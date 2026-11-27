mod protocol;

// 注意：`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`
// 只能待在 main.rs 里（那是 bin 专属的 crate 属性），写在这里会让 lib 编译不过。

use secback_core::format::{now_filetime, Account, Project, MAX_SLOTS};
use secback_core::merge::{MergeAction, MergeOptions};
use secback_core::vault::{make_id, Store};
use secback_core::{ProbeStatus};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
// 菜单是桌面专属 API：Tauri 移动端根本没有 `tauri::menu` 这个模块。
// Android 由抽屉/底栏提供操作；Windows 由主题菜单提供操作。
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::menu::CheckMenuItem;
#[cfg(not(any(target_os = "android", target_os = "ios", target_os = "windows")))]
use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_fs::{FsExt, OpenOptions};
use std::io::{Read, Write};
use zeroize::Zeroizing;

/// 复制到剪贴板后多久自动清空。密码不应该长期躺在剪贴板里。
#[cfg(not(any(target_os="android",target_os="ios")))]
const CLIP_CLEAR_SECS: u64 = 60;

/// 连续错几次之后开始加冷却。
const FAIL_FREE_TRIES: u32 = 5;
const FAIL_STEP_MS: u64 = 5000;

/// 闲置多久自动锁定。密码本放着不管应该自己关上，而不是一直开着明文。
/// 环境变量 SECBACK_IDLE_SECS 可以缩短它，只为自动化测试用——正常启动不存在。
fn idle_lock_duration() -> Duration {
    if let Ok(v) = std::env::var("SECBACK_IDLE_SECS") {
        if let Ok(n) = v.parse::<u64>() {
            return Duration::from_secs(n.max(3));
        }
    }
    Duration::from_secs(5 * 60)
}

/// 看门狗多久查一次。不需要精确到秒——多等 20 秒不算问题。
const WATCHDOG_TICK: Duration = Duration::from_secs(20);

/// 「外观」菜单那两个可勾选项的句柄。
/// 桌面端是真正的菜单对象；移动端没有菜单，用 `()` 占位，
/// 这样 `AppState` 的定义两端可以共用一份，不用整块复制。
#[cfg(not(any(target_os = "android", target_os = "ios")))]
type ThemeChecks = (CheckMenuItem<tauri::Wry>, CheckMenuItem<tauri::Wry>);
#[cfg(any(target_os = "android", target_os = "ios"))]
type ThemeChecks = ();

struct AppState {
    action: Mutex<()>,
    session: Mutex<u64>,
    recovering: Mutex<bool>,
    store: Mutex<Store>,
    path: PathBuf,
    selected: Mutex<String>,
    imported: Mutex<Vec<String>>,
    using_default_pwd: Mutex<bool>,
    fails: Mutex<u32>,
    retry_until: Mutex<Option<Instant>>,
    picking: std::sync::atomic::AtomicBool,
    import_request: std::sync::atomic::AtomicU64,
    /// 最后一次"有人动过"的时刻。前端会节流上报 touch，收到任何消息也会刷新。
    last_activity: Mutex<Instant>,
    /// 正在导入的那个外部密库：已通过预检、可能已解密，等用户确认。
    pending: Mutex<Option<PendingImport>>,
    /// 启动时配置解析失败的原因。有值时挡住登录，别让用户在错误的库上开始写。
    config_error: Option<String>,
    /// 当前是不是深色。前端存在 localStorage 里，启动后会同步过来；
    /// 这里存一份是为了菜单重建时能恢复对勾位置。
    dark_theme: Mutex<bool>,
    /// 「外观」里那两个可勾选项的句柄。
    /// 必须存句柄：Menu::get() 只查顶层项，钻不进子菜单里。
    #[allow(dead_code)]
    theme_checks: Mutex<Option<ThemeChecks>>,
}

/// 导入流程的中间状态；不保留解锁口令，临时项目在结束、取消或锁定时析构擦除。
struct PendingImport {
    label: String,
    ciphertext: Vec<u8>,
    incoming: Vec<Project>,
}

impl Drop for PendingImport {
    fn drop(&mut self) {
        self.incoming.clear();
    }
}

/// 锁被 poison 也要能继续用：密码本不该因为一次 panic 就整个废掉。
macro_rules! lock {
    ($m:expr) => {
        $m.lock().unwrap_or_else(|e| e.into_inner())
    };
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 密库位置：先看 SECBACK_VAULT（测试/调试用），否则按 secback.cfg 的规则解析。
///
/// 配置写错时**不静默退回默认**——那样用户会以为打开的是原来那个库，
/// 实际却新建了一个空库。这里把错误带出去，界面上明确显示并挡住登录。
fn resolve_vault_path(dir: &Path) -> (PathBuf, Option<String>) {
    if let Ok(p) = std::env::var("SECBACK_VAULT") {
        if p.trim().is_empty() || p.contains(['\n','\r']) { return (dir.join("sec.properties"),Some("SECBACK_VAULT 必须为非空的单行文件路径".into())); }
        if !p.is_empty() {
            return match secback_core::config::resolve_with(dir, Some(&format!("vault={}",p)), &|k| std::env::var(k).ok()) { Ok(r)=>(r.path,None), Err(e)=>(dir.join("sec.properties"),Some(e)) };
        }
    }
    match secback_core::config::resolve(dir) {
        Ok(r) => (r.path, None),
        Err(msg) => (dir.join("sec.properties"), Some(msg)),
    }
}

fn hex(id: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in id {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn parse_hex(s: &str) -> Option<[u8; 16]> { protocol::parse_id(s) }

fn push(app: &AppHandle, v: Value) {
    let _ = app.emit("host", v.to_string());
}

fn toast(app: &AppHandle, kind: &str, text: &str) {
    push(app, json!({ "t": "toast", "kind": kind, "text": text }));
}

fn build_snapshot(st: &AppState, store: &Store) -> Value {
    let sel = lock!(st.selected).clone();
    let imported = lock!(st.imported).clone();
    let list = store.projects();
    let idx = list.iter().position(|p| hex(&p.id) == sel);

    let projects: Vec<Value> = list
        .iter()
        .map(|p| {
            let id = hex(&p.id);
            let slots: Vec<Value> = (0..MAX_SLOTS)
                .map(|i| {
                    let s = &p.slots[i];
                    json!({
                        "user": s.user,
                        "remark": s.remark,
                        "hasPass": !s.pass.is_empty(),
                        "empty": s.is_empty(),
                    })
                })
                .collect();
            json!({
                "id": id,
                "name": p.name,
                "remark": p.remark,
                "imported": imported.contains(&id),
                "slots": slots,
            })
        })
        .collect();

    json!({
        "t": "snapshot",
        "projects": projects,
        "selected": sel,
        "canUp": idx.map(|i| i > 0).unwrap_or(false),
        "canDown": idx.map(|i| i + 1 < list.len()).unwrap_or(false),
        "defaultPwd": *lock!(st.using_default_pwd),
    })
}

fn refresh(app: &AppHandle, st: &AppState, store: &Store) {
    push(app, build_snapshot(st, store));
}

/// 保存并刷新。写盘失败就只报错、不动界面。
fn persist(app: &AppHandle, st: &AppState, store: &mut Store, candidate: Store, selected: String, ok_text: &str) -> bool {
    match store.commit(&st.path, candidate) {
        Ok(()) => {
            *lock!(st.selected) = selected;
            refresh(app, st, store);
            if !ok_text.is_empty() {
                toast(app, "ok", ok_text);
            }
            true
        }
        Err(e) => {
            toast(app, "err", &format!("保存失败：{}", e.text()));
            false
        }
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn copy_to_clipboard(_app: &AppHandle, text: &str) -> Result<(), String> {
    let owned = Zeroizing::new(text.to_string());
    let mut cb = arboard::Clipboard::new().map_err(|_| "无法访问系统剪贴板")?;
    cb.set_text(owned.to_string()).map_err(|_| "写入剪贴板失败")?;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(CLIP_CLEAR_SECS));
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Ok(cur) = cb.get_text() {
                let cur = Zeroizing::new(cur);
                if *cur == *owned { let _ = cb.clear(); }
            }
        }
    });
    Ok(())
}
#[cfg(target_os = "android")]
struct Native(tauri::plugin::PluginHandle<tauri::Wry>);
#[cfg(target_os = "android")]
fn copy_to_clipboard(app: &AppHandle, text: &str) -> Result<(), String> {
    app.state::<Native>().0.run_mobile_plugin::<Value>("copy", json!({"text":text}))
        .map(|_| ()).map_err(|_| "写入系统剪贴板失败".into())
}
#[cfg(target_os = "ios")]
fn copy_to_clipboard(_app: &AppHandle, _text: &str) -> Result<(), String> { Err("此平台暂不支持复制".into()) }

fn retry_remaining(st: &AppState) -> u64 {
    lock!(st.retry_until).map(|t| t.saturating_duration_since(Instant::now()).as_millis() as u64).unwrap_or(0)
}
fn auth_failed(st: &AppState) -> u64 {
    let mut fails = lock!(st.fails);
    *fails = fails.saturating_add(1);
    let ms = if *fails < FAIL_FREE_TRIES { 0 } else { ((*fails - FAIL_FREE_TRIES + 1) as u64 * FAIL_STEP_MS).min(60_000) };
    *lock!(st.retry_until) = Some(Instant::now() + Duration::from_millis(ms));
    ms
}
fn valid_new_password(pwd: &str) -> bool { (6..=1024).contains(&pwd.chars().count()) }

fn apply_login(app: &AppHandle, st: &AppState, pwd: &str, confirm: &str) {
    let wait = retry_remaining(st);
    if wait > 0 { push(app,json!({"t":"loginError","text":"请等待后重试","retryAfter":wait})); return; }
    if !st.path.exists() && (!valid_new_password(pwd) || pwd != confirm) {
        push(app,json!({"t":"loginError","text":"创建密库请填写两次一致的主密码（6～1024 字符）","retryAfter":0})); return;
    }
    let mut store = lock!(st.store);

    match store.load_or_create(&st.path, pwd, true) {
        Ok(_) => {
            *lock!(st.session) += 1;
            *lock!(st.recovering) = false;
            *lock!(st.fails) = 0;
            *lock!(st.retry_until) = None;
            *lock!(st.using_default_pwd) = pwd == "1021";

            let first = store.projects().first().map(|p| hex(&p.id)).unwrap_or_default();
            *lock!(st.selected) = first;

            // 先发数据、再切视图：反过来会先看到空列表闪一下。
            refresh(app, st, &store);
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            install_menu(app, true);
            push(app, json!({ "t": "loggedIn" }));
        }
        Err(e) => {
            let retry_after = auth_failed(st);
            push(
                app,
                json!({ "t": "loginError", "text": e.text(), "retryAfter": retry_after }),
            );
        }
    }
}

/// 锁定：抹掉内存里的密钥与模型，把界面切回登录页。
fn do_lock(app: &AppHandle, st: &AppState) {
    *lock!(st.session) += 1;
    {
        let mut store = lock!(st.store);
        store.clear_secure();
    }
    *lock!(st.pending) = None;
    *lock!(st.recovering) = false;
    *lock!(st.using_default_pwd) = false;
    *lock!(st.selected) = String::new();
    *lock!(st.imported) = Vec::new();
    // 重置计时，否则刚锁完又被判定为闲置
    *lock!(st.last_activity) = Instant::now();
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    install_menu(app, false);
    push(app, json!({ "t": "loggedOut" }));

    // 故意不动窗口：闲置锁定多半发生在用户已经跑去干别的事了，
    // 这时候把窗口弹到最前面是打扰。用户回来点开，看到的就是登录页。
}

/// 闲置看门狗：跑在独立线程里，不占 UI 线程。
///
/// 为什么不放在前端算：窗口最小化时浏览器的定时器会被大幅节流，
/// 「离开一小时回来还是解锁状态」正是最危险的情况。计时必须在宿主侧。
fn spawn_idle_watchdog(handle: AppHandle) {
    let idle_limit = idle_lock_duration();
    // 查询间隔不能比限制本身还长，否则测试用的短时长会被拖到 20 秒
    let tick = WATCHDOG_TICK.min(idle_limit / 2).max(Duration::from_secs(1));

    std::thread::spawn(move || loop {
        std::thread::sleep(tick);
        let st = handle.state::<AppState>();
        let _action = lock!(st.action);
        let expired = {
            let unlocked = lock!(st.store).is_unlocked();
            unlocked && lock!(st.last_activity).elapsed() >= idle_limit
        };
        if expired {
            do_lock(&handle, st.inner());
        }
    });
}

// ---------------------------------------------------------------- 跨机器导入

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

/// 同名比较用的键：去掉首尾空白（含全角空格，中文输入法很容易打出来）。
fn name_key(s: &str) -> String {
    s.trim_matches(|c: char| {
        c == ' ' || c == '\t' || c == '\r' || c == '\n' || c == '\u{3000}'
    })
    .to_string()
}

/// 第一步：有界读取密文快照并预检，不需要口令。
/// 先告诉用户"能不能读"，再要密码——而不是输了密码才发现版本不对。
fn start_import(app: &AppHandle, st: &AppState, path: PathBuf) {
    if !lock!(st.store).is_unlocked() || *lock!(st.recovering) { toast(app,"err","请先完成解锁"); return; }
    if path == st.path {
        toast(app, "err", "这就是当前正在使用的密库，不需要导入");
        return;
    }
    if !lock!(st.store).is_unlocked() {
        toast(app, "err", "请先解锁再导入");
        return;
    }

    st.import_request.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let bytes = match std::fs::File::open(&path).and_then(read_import_bytes) {
        Ok(bytes) => bytes,
        Err(e) => { toast(app, "err", &format!("读取外部密库失败：{}", e)); return; }
    };
    start_import_bytes(app, st, file_label(&path), bytes);
}

fn read_import_bytes(reader: impl Read) -> std::io::Result<Vec<u8>> {
    let max = secback_core::format::MAX_PAYLOAD_BYTES + 272;
    let mut bytes = Vec::new();
    reader.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max { return Err(std::io::Error::other("密库文件超过 64 MiB 负载上限")); }
    Ok(bytes)
}

fn start_import_bytes(app: &AppHandle, st: &AppState, label: String, bytes: Vec<u8>) {
    if !lock!(st.store).is_unlocked() || *lock!(st.recovering) { return; }
    let info = secback_core::vault::probe_bytes(&bytes);
    let why = match info.status {
        ProbeStatus::Ok => {
            *lock!(st.pending) = Some(PendingImport {
                label: label.clone(),
                ciphertext: bytes,
                incoming: Vec::new(),
            });
            push(
                app,
                json!({ "t": "importNeedPassword", "fileName": label }),
            );
            return;
        }
        ProbeStatus::Missing => "文件不存在".to_string(),
        ProbeStatus::Io => "打不开这个文件（可能被占用或没有权限）".to_string(),
        ProbeStatus::TooSmall => "文件太小，不是密库".to_string(),
        ProbeStatus::NotVault => "这不是 SecBack 的密库文件".to_string(),
        ProbeStatus::VersionNewer(v) => {
            format!("这个密库是更新版本（v{}）建的，当前程序读不了", v)
        }
        ProbeStatus::VersionOlder(v) => format!("这个密库的格式版本（v{}）太旧", v),
        ProbeStatus::Corrupt => "文件头损坏，无法读取".to_string(),
    };
    toast(app, "err", &why);
}

/// 第二步：用对方的口令解密，算出合并计划并推给前端预览。
fn import_unlock(app: &AppHandle, st: &AppState, pwd: &str) {
    let result = {
        let pending = lock!(st.pending);
        let Some(p) = pending.as_ref() else { return; };
        secback_core::vault::load_from_bytes(&p.ciphertext, pwd)
    };
    match result {
        Ok(incoming) => {
            if incoming.is_empty() {
                *lock!(st.pending) = None;
                toast(app, "err", "这个密库是空的，没有可导入的内容");
                return;
            }
            if let Some(p) = lock!(st.pending).as_mut() {
                p.incoming = incoming;
            }
            push_import_preview(app, st);
        }
        Err(e) => push(app, json!({ "t": "importError", "text": e.text() })),
    }
}

/// 预览：把合并计划摊平成前端好渲染的树。
/// 注意这里**不传密码**，只有 hasPass——和主界面保持同一套原则。
fn push_import_preview(app: &AppHandle, st: &AppState) {
    let pend = lock!(st.pending);
    let Some(p) = pend.as_ref() else { return };

    let store = lock!(st.store);
    let local = store.projects();

    // 默认策略：id 对得上就并进同一个项目；id 对不上但名字相同，也默认合并，
    // 用户可以在预览里逐条取消。
    let mut opt = MergeOptions::default();
    for inc in &p.incoming {
        if local.iter().any(|x| x.id == inc.id) {
            continue;
        }
        let want = name_key(&inc.name);
        if !want.is_empty() && local.iter().any(|x| name_key(&x.name) == want) {
            opt.merge_same_name.push(inc.id);
        }
    }

    let plan = secback_core::merge::merge(local, &p.incoming, &opt);

    let entries: Vec<Value> = plan
        .entries
        .iter()
        .map(|e| {
            json!({
                "id": hex(&e.incoming_id),
                "name": e.name,
                "action": match e.action {
                    MergeAction::Add => "add",
                    MergeAction::Merge => "merge",
                    MergeAction::Skip => "skip",
                },
                "sameName": e.same_name,
                "localName": if e.local_index >= 0 {
                    local.get(e.local_index as usize).map(|x| x.name.clone()).unwrap_or_default()
                } else {
                    String::new()
                },
                "copies": e.copies,
                "metadataConflict": e.metadata_conflict,
                "incomingRemark": p.incoming.iter().find(|x|x.id==e.incoming_id).map(|x|x.remark.as_str()).unwrap_or(""),
                "localRemark": if e.local_index>=0 { local[e.local_index as usize].remark.as_str() } else { "" },
                "slots": e.slots.iter().map(|s| json!({
                    "user": s.acc.user,
                    "remark": s.acc.remark,
                    "hasPass": !s.acc.pass.is_empty(),
                    "srcSlot": s.src_slot,
                    "localSlot": s.local_slot,
                    "duplicate": s.duplicate,
                    "excluded": s.excluded,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    push(
        app,
        json!({
            "t": "importPreview",
            "fileName": p.label,
            "entries": entries,
            "summary": {
                "added": plan.added,
                "merged": plan.merged_count,
                "slotsAdded": plan.slots_added,
                "overflow": plan.overflow_count,
                "metadataChanged": plan.metadata_changed,
            },
        }),
    );
}

fn parse_id_list(v: Option<&Value>) -> Vec<[u8; 16]> {
    v.and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().and_then(parse_hex))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_slot_list(v: Option<&Value>) -> Vec<([u8; 16], u8)> {
    v.and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    let id = o.get("id")?.as_str().and_then(parse_hex)?;
                    let slot = u8::try_from(o.get("slot")?.as_u64()?).ok()?;
                    if slot >= MAX_SLOTS as u8 { return None; }
                    Some((id, slot))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 第三步：按用户勾选的结果重新算一遍，先写盘成功再换内存。
fn import_commit(app: &AppHandle, st: &AppState, msg: &Value) {
    // 一次性取走 pending，避免后面重复加锁；失败时也不保留（文件还在，重来即可）
    let Some(pending) = lock!(st.pending).take() else {
        return;
    };

    let opt = MergeOptions {
        merge_same_name: parse_id_list(msg.get("mergeSameName")),
        skip_projects: parse_id_list(msg.get("skipProjects")),
        exclude_slots: parse_slot_list(msg.get("excludeSlots")),
        use_incoming_metadata: parse_id_list(msg.get("useIncomingMetadata")),
    };

    let mut store = lock!(st.store);
    let before = store.projects().len();
    let plan = secback_core::merge::merge(store.projects(), &pending.incoming, &opt);

    if plan.slots_added == 0 && plan.added == 0 && plan.metadata_changed == 0 {
        push(app,json!({"t":"importResult","ok":true}));
        toast(app, "info", "本次选项没有需要导入的内容");
        return;
    }

    // 写盘成功之前绝不改内存：任何一步失败，磁盘和内存都还是原样。
    if let Err(e) = store.write_out(&st.path, &plan.merged) {
        toast(
            app,
            "err",
            &format!("导入失败，密库没有任何改动：{}", e.text()),
        );
        *lock!(st.pending)=Some(pending);
        push(app,json!({"t":"importResult","ok":false,"text":e.text()}));
        return;
    }

    let tail = before.min(plan.merged.len());
    let new_ids: Vec<String> = plan.merged[tail..].iter().map(|x| hex(&x.id)).collect();

    *store.projects_mut() = plan.merged;
    *lock!(st.imported) = new_ids.clone();
    if let Some(first) = new_ids.first() {
        *lock!(st.selected) = first.clone();
    }

    refresh(app, st, &store);
    drop(store);

    let mut text = format!(
        "导入完成：新增 {} 个项目、{} 套账户",
        plan.added, plan.slots_added
    );
    if plan.overflow_count > 0 {
        text.push_str(&format!(
            "；另存 {} 个带「（导入）」后缀的副本",
            plan.overflow_count
        ));
    }
    if plan.metadata_changed>0 { text.push_str(&format!("；更新 {} 个项目的名称/备注",plan.metadata_changed)); }
    push(app,json!({"t":"importResult","ok":true}));
    toast(app, "ok", &text);
}

// ---------------------------------------------------------------- 菜单

// Windows renders themed menus in the WebView and handles shortcuts there.
#[cfg(target_os = "windows")]
fn install_menu(_app: &AppHandle, _unlocked: bool) {}

#[cfg(not(any(target_os = "android", target_os = "ios", target_os = "windows")))]
fn install_menu(app: &AppHandle, unlocked: bool) {
    let result = (|| -> tauri::Result<()> {
        let mut file = SubmenuBuilder::new(app, "文件");

        if unlocked {
            file = file
                .item(
                    &MenuItemBuilder::with_id("new", "新增项目")
                        .accelerator("CmdOrCtrl+N")
                        .build(app)?,
                )
                .item(
                    &MenuItemBuilder::with_id("delete", "删除项目")
                        .accelerator("Delete")
                        .build(app)?,
                )
                .separator()
                .item(&MenuItemBuilder::with_id("import", "从另一台机器导入…").build(app)?)
                .item(&MenuItemBuilder::with_id("export", "导出加密备份…").build(app)?)
                .separator()
                .item(&MenuItemBuilder::with_id("changePwd", "修改主密码…").build(app)?)
                .item(&MenuItemBuilder::with_id("setRecovery", "设置恢复码…").build(app)?)
                .separator()
                .item(
                    &MenuItemBuilder::with_id("lock", "锁定")
                        .accelerator("CmdOrCtrl+L")
                        .build(app)?,
                )
                .separator();
        }

        file = file.item(
            &MenuItemBuilder::with_id("quit", "退出")
                .accelerator("CmdOrCtrl+Q")
                .build(app)?,
        );

        // 菜单会被反复重建（登录/锁定时），勾选状态要从 state 里读回来，
        // 否则切一次主题再锁一次，对勾就没了。
        let dark_now = *lock!(app.state::<AppState>().dark_theme);

        let dark_item = CheckMenuItemBuilder::with_id("theme-dark", "深色")
            .checked(dark_now)
            .build(app)?;
        let light_item = CheckMenuItemBuilder::with_id("theme-light", "浅色")
            .checked(!dark_now)
            .build(app)?;

        let view = SubmenuBuilder::new(app, "外观")
            .item(&dark_item)
            .item(&light_item)
            .build()?;

        // 把句柄存起来，后面切换主题时直接拨这两项
        if let Some(st) = app.try_state::<AppState>() {
            *lock!(st.theme_checks) = Some((dark_item.clone(), light_item.clone()));
        }

        let help = SubmenuBuilder::new(app, "帮助")
            .item(&MenuItemBuilder::with_id("about", "关于 SecBack").build(app)?)
            .build()?;

        let menu = MenuBuilder::new(app)
            .items(&[&file.build()?, &view, &help])
            .build()?;

        app.set_menu(menu)?;
        Ok(())
    })();

    if let Err(e) = result {
        eprintln!("菜单创建失败：{}", e);
    }
}

/// 窗口标题栏跟界面保持一致。深色界面顶个白标题栏太割裂。
fn set_window_theme(app: &AppHandle, dark: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_theme(Some(if dark {
            tauri::Theme::Dark
        } else {
            tauri::Theme::Light
        }));
    }
}

/// 让「外观」菜单里的对勾跟当前主题一致。
/// 菜单不重建的时候勾选状态不会自己变，得手动拨。
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn set_theme_checks(app: &AppHandle, dark: bool) {
    let Some(st) = app.try_state::<AppState>() else {
        return;
    };
    let guard = lock!(st.theme_checks);
    if let Some((dark_item, light_item)) = guard.as_ref() {
        let _ = dark_item.set_checked(dark);
        let _ = light_item.set_checked(!dark);
    }
}

/// 主题变更的统一入口：记状态、调标题栏、拨对勾。
/// 注意这里**不发消息给前端**——发消息是在菜单点击那里做的，
/// 否则会和前端回报的 theme_sync 互相触发，绕成死循环。
fn apply_theme(app: &AppHandle, st: &AppState, dark: bool) {
    #[cfg(target_os="android")]
    { let _ = app.state::<Native>().0.run_mobile_plugin::<Value>("theme",json!({"dark":dark})); }
    *lock!(st.dark_theme) = dark;
    set_window_theme(app, dark);
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    set_theme_checks(app, dark);
}

fn pick_and_import(app: AppHandle) {
    let st = app.state::<AppState>();
    if !lock!(st.store).is_unlocked() || *lock!(st.recovering) {
        toast(&app, "err", "请先解锁再导入"); return;
    }
    if st.picking.swap(true, std::sync::atomic::Ordering::SeqCst) { return; }
    let opened = *lock!(st.session);
    let request = st.import_request.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let handle = app.clone();
    // Android 文档提供方通常把自定义扩展名归为 application/octet-stream，
    // 不按 MIME 过滤，交给密库文件头预检识别。
    let dialog = app.dialog().file().set_title("导入外部密库");
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let dialog = dialog.add_filter("SecBack 密库", &["properties", "property"]);
    dialog.pick_file(move |picked| {
        handle.state::<AppState>().picking.store(false, std::sync::atomic::Ordering::SeqCst);
        let Some(fp) = picked else { return; };
        std::thread::spawn(move || {
            // URI 解析可能回调 Android 主线程，读取期间不能持 action 锁。
            let result = (|| {
                let mut opts = OpenOptions::new(); opts.read(true);
                let file = handle.fs().open(fp.clone(), opts).map_err(|e| e.to_string())?;
                read_import_bytes(file).map_err(|e| e.to_string())
            })();
            let st = handle.state::<AppState>();
            let _action = lock!(st.action);
            if !lock!(st.store).is_unlocked() || *lock!(st.recovering) || opened != *lock!(st.session) {
                toast(&handle, "err", "密库已锁定，请解锁后重新导入"); return;
            }
            if request != st.import_request.load(std::sync::atomic::Ordering::SeqCst) { return; }
            if let Ok(path) = fp.clone().into_path() {
                if std::fs::canonicalize(&path).ok().zip(std::fs::canonicalize(&st.path).ok()).is_some_and(|(a,b)| a==b) {
                    toast(&handle, "err", "这就是当前正在使用的密库，不需要导入"); return;
                }
            }
            let label = fp.clone().into_path().map(|p| file_label(&p)).unwrap_or_else(|_| "所选外部密库".into());
            match result {
                Ok(bytes) => start_import_bytes(&handle, st.inner(), label, bytes),
                Err(e) => toast(&handle, "err", &format!("读取外部密库失败：{}", e)),
            }
        });
    });
}

/// 只复制已经成功落盘的密文，不把明文交给文件选择器。
fn pick_and_export(app: AppHandle) {
    let st = app.state::<AppState>();
    if !lock!(st.store).is_unlocked() || *lock!(st.recovering) {
        toast(&app, "err", "请先解锁再导出");
        return;
    }
    // 锁定、重新登录后不再使用旧文件选择结果。
    let opened = *lock!(st.session);
    st.picking.store(true, std::sync::atomic::Ordering::SeqCst);
    let handle = app.clone();
    app.dialog().file().set_title("导出加密备份")
        .set_file_name(format!("SecBack-备份-{}.properties", now_filetime()))
        .add_filter("SecBack 加密备份", &["properties"])
        .save_file(move |picked| {
            handle.state::<AppState>().picking.store(false, std::sync::atomic::Ordering::SeqCst);
            let Some(fp) = picked else { return; };
            // Android 文件描述符解析涉及主线程插件，放到工作线程。
            std::thread::spawn(move || {
                let st = handle.state::<AppState>();
                let _action = lock!(st.action);
                if !lock!(st.store).is_unlocked() || *lock!(st.recovering) || opened != *lock!(st.session) {
                    toast(&handle, "err", "密库已锁定，请解锁后重新导出");
                    return;
                }
                if let Ok(path) = fp.clone().into_path() {
                    if std::fs::canonicalize(&path).ok().zip(std::fs::canonicalize(&st.path).ok()).is_some_and(|(a,b)|a==b) {
                        toast(&handle, "err", "备份不能覆盖当前密库，请选择其他文件");
                        return;
                    }
                }
                let result = (|| -> Result<(), String> {
                    let bytes = std::fs::read(&st.path).map_err(|e| e.to_string())?;
                    let mut opts = OpenOptions::new();
                    opts.write(true).create(true).truncate(true);
                    let mut file = handle.fs().open(fp.clone(), opts).map_err(|e| e.to_string())?;
                    file.write_all(&bytes).map_err(|e| e.to_string())?;
                    file.flush().map_err(|e| e.to_string())?;
                    file.sync_all().map_err(|e| e.to_string())?;
                    drop(file);
                    let mut read_opts = OpenOptions::new();
                    read_opts.read(true);
                    let mut readback = Vec::new();
                    handle.fs().open(fp.clone(), read_opts).map_err(|e| e.to_string())?
                        .take(bytes.len() as u64 + 1).read_to_end(&mut readback).map_err(|e| e.to_string())?;
                    if bytes != readback { return Err("备份回读校验不一致".into()); }
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        push(&handle, json!({"t":"exportResult", "destination":fp.to_string()}));
                        toast(&handle, "ok", "加密备份已导出，回读校验通过");
                    },
                    Err(e) => toast(&handle, "err", &format!("导出失败，备份未确认有效：{}", e)),
                }
            });
        });
}

fn handle(app: &AppHandle, st: &AppState, msg: &Value) {
    let t = msg["t"].as_str().unwrap_or("");

    // 只要前端还发消息，就说明有人在动
    *lock!(st.last_activity) = Instant::now();

    let public = matches!(t, "ready" | "login" | "recover" | "recover_set_password" | "recover_cancel" | "theme_sync" | "quit" | "about" | "lock" | "background" | "touch");
    if (!public && (!lock!(st.store).is_unlocked() || *lock!(st.recovering)))
        || (t == "recover_set_password" && !*lock!(st.recovering)) {
        if matches!(t, "create" | "save") {
            push(app, json!({"t":"saveResult", "request":msg["request"], "ok":false}));
        }
        toast(app, "err", "请先完成解锁");
        return;
    }

    match t {
        "ready" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            let first_run = !st.path.exists();
            push(
                app,
                json!({
                    "t": "bootstrap",
                    "firstRun": first_run,
                    "vaultPath": st.path.to_string_lossy(),
                    "configError": st.config_error,
                    "platform": std::env::consts::OS,
                    "version": app.package_info().version.to_string(),
                }),
            );
        }

        "login" => {
            // 配置文件写错就不让进：否则用户会在一个错误的位置新建空库，
            // 然后以为"数据丢了"。
            if let Some(err) = &st.config_error {
                push(
                    app,
                    json!({ "t": "loginError", "text": err, "retryAfter": 0 }),
                );
                return;
            }
            let pwd = Zeroizing::new(msg["pwd"].as_str().unwrap_or("").to_string());
            if !pwd.is_empty() {
                apply_login(app, st, &pwd, msg["confirm"].as_str().unwrap_or(""));
            }
        }

        "select" => {
            let id = msg["id"].as_str().unwrap_or("");
            if parse_hex(id).is_some() {
                *lock!(st.selected) = id.to_string();
                let store = lock!(st.store);
                if let Some(idx) = parse_hex(id).and_then(|h| store.find_index(&h)) {
                    push(app, json!({"t":"selection","selected":id,"canUp":idx>0,"canDown":idx+1<store.projects().len()}));
                }
            }
        }

        "reveal" => {
            let id = msg["id"].as_str().unwrap_or("");
            let slot = msg["slot"].as_u64().unwrap_or(99) as usize;
            let store = lock!(st.store);
            if let Some(idx) = parse_hex(id).and_then(|h| store.find_index(&h)) {
                if slot < MAX_SLOTS {
                    push(
                        app,
                        json!({
                            "t": "secret",
                            "id": id,
                            "slot": slot,
                            "request": msg["request"],
                            "pass": store.projects()[idx].slots[slot].pass,
                        }),
                    );
                }
            }
        }

        "copy" => {
            let id = msg["id"].as_str().unwrap_or("");
            let slot = msg["slot"].as_u64().unwrap_or(99) as usize;
            let field = msg["field"].as_str().unwrap_or("pass");
            let store = lock!(st.store);
            if let Some(idx) = parse_hex(id).and_then(|h| store.find_index(&h)) {
                if slot < MAX_SLOTS {
                    let s = &store.projects()[idx].slots[slot];
                    let text = if field == "user" { &s.user } else { &s.pass };
                    if !text.is_empty() {
                        match copy_to_clipboard(app, text) {
                            Ok(()) => toast(app,"ok",if cfg!(target_os="android") { "已复制；前台 60 秒后尝试清除，切到后台会立即尝试清除" } else { "已复制，60 秒后尝试清除（仅清除本次内容）" }),
                            Err(e) => toast(app,"err",&e),
                        }
                    }
                }
            }
        }

        "up" | "down" => {
            let dir: i64 = if t == "up" { -1 } else { 1 };
            let sel = lock!(st.selected).clone();
            let mut store = lock!(st.store);
            if let Some(idx) = parse_hex(&sel).and_then(|h| store.find_index(&h)) {
                let target = idx as i64 + dir;
                if target >= 0 && (target as usize) < store.projects().len() {
                    let mut candidate = store.staged();
                    candidate.move_item(idx, target as usize);
                    persist(app, st, &mut store, candidate, sel, "");
                }
            }
        }

        "reorder" => {
            // 拖拽排序专用：直接把项目挪到目标下标，一次落盘。
            // 不用逐条发 up/down——拖过十几项会触发十几次渲染和写盘，卡得很明显。
            let id = msg["id"].as_str().unwrap_or("");
            let to = msg["to"].as_u64().unwrap_or(0) as usize;
            let mut store = lock!(st.store);
            if let Some(idx) = parse_hex(id).and_then(|h| store.find_index(&h)) {
                if to < store.projects().len() && idx != to {
                    let mut candidate = store.staged();
                    candidate.move_item(idx, to);
                    let selected = lock!(st.selected).clone();
                    persist(app, st, &mut store, candidate, selected, "");
                }
            }
        }

        "delete" => {
            let sel = msg["id"].as_str().unwrap_or("").to_string();
            let mut current = lock!(st.store);
            let mut store = current.staged();
            if let Some(idx) = parse_hex(&sel).and_then(|h| store.find_index(&h)) {
                store.projects_mut().remove(idx);
                // 删完选下一个，没有就选上一个
                let n = store.projects().len();
                let next = if n == 0 {
                    String::new()
                } else {
                    hex(&store.projects()[idx.min(n - 1)].id)
                };
                persist(app, st, &mut current, store, next, "已删除");
            }
        }

        "create" | "save" => {
            let is_new = t == "create";
            let mut current = lock!(st.store);
            let mut store = current.staged();

            let idx = if is_new {
                let Some(id) = make_id() else {
                    push(app, json!({"t":"saveResult", "request":msg["request"], "ok":false}));
                    toast(app,"err","无法生成项目标识，请重试");
                    return;
                };
                store.projects_mut().push(Project::new_with_id(id));
                store.projects().len() - 1
            } else {
                match parse_hex(msg["id"].as_str().unwrap_or(""))
                    .and_then(|h| store.find_index(&h))
                {
                    Some(i) => i,
                    None => {
                        push(app, json!({"t":"saveResult", "request":msg["request"], "ok":false}));
                        toast(app,"err","项目已不存在，请重新打开编辑");
                        return;
                    },
                }
            };

            let new_id = {
                let p = &mut store.projects_mut()[idx];
                p.name = msg["name"].as_str().unwrap_or("").to_string();
                if p.name.trim().is_empty() {
                    p.name = "未命名".to_string();
                }
                p.remark = msg["remark"].as_str().unwrap_or("").to_string();
                p.modified_utc = now_filetime();

                let originals = p.slots.clone();
                for i in 0..MAX_SLOTS {
                    let key = format!("s{}", i);
                    match msg.get(&key) {
                        Some(sj) => {
                            // pass 三态：缺省/null = 不动；"" = 清空；其他 = 替换
                            let source = sj["source"].as_u64().map(|n| n as usize).unwrap_or(i);
                            let keep = originals.get(source).map(|s| s.pass.clone()).unwrap_or_default();
                            p.slots[i] = Account {
                                user: sj["user"].as_str().unwrap_or("").to_string(),
                                remark: sj["remark"].as_str().unwrap_or("").to_string(),
                                pass: match sj.get("pass") {
                                    Some(Value::String(s)) => s.clone(),
                                    _ => keep,
                                },
                            };
                        }
                        None => p.slots[i] = Account::empty(),
                    }
                }
                hex(&p.id)
            };

            let ok = persist(
                app,
                st,
                &mut current,
                store,
                new_id,
                if is_new { "已新增项目" } else { "已保存" },
            );
            push(app, json!({"t":"saveResult", "request":msg["request"], "ok":ok}));
        }

        "lock" => do_lock(app, st),
        "background" => { if !st.picking.load(std::sync::atomic::Ordering::SeqCst) { do_lock(app,st); } },

        "import" => pick_and_import(app.clone()),
        "export" => pick_and_export(app.clone()),

        // 拖放进来时前端直接把路径发过来（Tauri 的拖放事件在宿主侧，这里只是兜底）
        "import_path" => {
            let p = msg["path"].as_str().unwrap_or("");
            if !p.is_empty() {
                start_import(app, st, PathBuf::from(p));
            }
        }

        "import_unlock" => {
            let pwd = Zeroizing::new(msg["pwd"].as_str().unwrap_or("").to_string());
            if !pwd.is_empty() {
                import_unlock(app, st, &pwd);
            }
        }

        "import_plan" => {
            let pending = lock!(st.pending); let Some(p) = pending.as_ref() else {return;};
            let store = lock!(st.store);
            let opt = MergeOptions {
                merge_same_name:parse_id_list(msg.get("mergeSameName")), skip_projects:parse_id_list(msg.get("skipProjects")),
                exclude_slots:parse_slot_list(msg.get("excludeSlots")),use_incoming_metadata:parse_id_list(msg.get("useIncomingMetadata")),
            };
            let plan=secback_core::merge::merge(store.projects(),&p.incoming,&opt);
            push(app,json!({"t":"importPlan","request":msg["request"],"summary":{"added":plan.added,"slotsAdded":plan.slots_added,"overflow":plan.overflow_count,"metadataChanged":plan.metadata_changed}}));
        }
        "import_commit" => import_commit(app, st, msg),

        "import_cancel" => {
            st.import_request.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *lock!(st.pending) = None;
        }

        "rekey" => {
            let old = Zeroizing::new(msg["old"].as_str().unwrap_or("").to_string());
            let new = Zeroizing::new(msg["new"].as_str().unwrap_or("").to_string());
            if !valid_new_password(&new) {
                toast(app, "err", "新主密码须为 6～1024 字符");
                return;
            }
            let mut store = lock!(st.store);
            match store.rekey(&st.path, &old, &new) {
                Ok(()) => {
                    *lock!(st.using_default_pwd) = new.as_str() == "1021";
                    refresh(app, st, &store);
                    push(app, json!({"t":"rekeyResult", "ok":true}));
                    toast(app, "ok", "主密码已更新，下次用新密码进入");
                }
                Err(e) => toast(app, "err", &format!("修改失败：{}", e.text())),
            }
        }

        // 设恢复码：验证当前口令 → 生成一组新码（首次会顺带把库升到 v2）
        "set_recovery" => {
            let pwd = Zeroizing::new(msg["pwd"].as_str().unwrap_or("").to_string());
            let mut store = lock!(st.store);
            match store.enable_recovery(&st.path, &pwd) {
                Ok(code) => {
                    refresh(app, st, &store);
                    push(app, json!({ "t": "recoveryCode", "code": code }));
                }
                Err(e) => push(app, json!({ "t": "recoveryError", "text": e.text() })),
            }
        }

        // 用恢复码解锁。成功后先不让人进去，必须设好新密码才放行——
        // 否则他改了数据一保存，库里存的还是那把忘掉的口令。
        "recover" => {
            let wait = retry_remaining(st);
            if wait > 0 { push(app,json!({"t":"recoverError","text":format!("请等待 {} 秒后重试",wait.div_ceil(1000))})); return; }
            let code = Zeroizing::new(msg["code"].as_str().unwrap_or("").to_string());
            let mut store = lock!(st.store);
            match store.recover(&st.path, &code) {
                Ok(()) => {
                    *lock!(st.recovering) = true;
                    *lock!(st.fails) = 0;
                    *lock!(st.retry_until) = None;
                    let n = store.projects().len();
                    push(app, json!({ "t": "recoverOk", "items": n }));
                }
                Err(e) => { auth_failed(st); push(app, json!({ "t": "recoverError", "text": e.text() })); },
            }
        }

        "recover_set_password" => {
            let new = Zeroizing::new(msg["new"].as_str().unwrap_or("").to_string());
            if !valid_new_password(&new) { push(app,json!({"t":"recoverError","text":"新主密码须为 6～1024 字符"})); return; }
            let mut store = lock!(st.store);
            match store.set_password(&st.path, &new) {
                Ok(()) => {
                    *lock!(st.recovering) = false;
                    *lock!(st.using_default_pwd) = new.as_str() == "1021";
                    *lock!(st.selected) = store
                        .projects()
                        .first()
                        .map(|p| hex(&p.id))
                        .unwrap_or_default();
                    refresh(app, st, &store);
                    #[cfg(not(any(target_os = "android", target_os = "ios")))]
                    install_menu(app, true);
                    push(app, json!({ "t": "loggedIn" }));
                    toast(app, "ok", "新主密码已生效。恢复码仍然可用。");
                }
                Err(e) => push(app, json!({ "t": "recoverError", "text": e.text() })),
            }
        }

        // 恢复途中取消：把解出来的东西重新抹掉
        "recover_cancel" => {
            do_lock(app, st);
        }

        // 前端恢复上次选的外观时告诉宿主。这里只更新宿主状态、不回消息——
        // 前端那边已经应用过了，回一条 cmd 会和它再回报一次，绕成死循环。
        "theme_sync" => {
            apply_theme(app, st, msg["dark"].as_bool().unwrap_or(true));
        }

        #[cfg(target_os = "windows")]
        "quit" => app.exit(0),

        "about" => {
            toast(
                app,
                "ok",
                &format!("SecBack {} · 本地密码本 · 请定期导出加密备份",app.package_info().version),
            );
        }

        _ => {}
    }
}

fn wipe_json(value: &mut Value) {
    match value {
        Value::String(s) => secback_core::crypto::wipe_string(s),
        Value::Array(a) => a.iter_mut().for_each(wipe_json),
        Value::Object(o) => o.values_mut().for_each(wipe_json),
        _ => {}
    }
}

#[tauri::command]
async fn host_message(app: AppHandle, json: String) -> Result<(), String> {
    // 密钥派生、文件 I/O 和 Android 文件插件都不能阻塞主线程。
    if json.len() > protocol::MAX_MESSAGE_BYTES { return Err("请求过大".into()); }
    tauri::async_runtime::spawn_blocking(move || {
        let json = Zeroizing::new(json);
        let mut msg = serde_json::from_str::<Value>(&json).map_err(|_| "请求格式错误".to_string())?;
        let result = protocol::validate(&msg);
        if result.is_ok() {
            let state = app.state::<AppState>();
            let _action = lock!(state.action);
            handle(&app, &state, &msg);
        }
        wipe_json(&mut msg);
        result
    }).await.map_err(|_| "请求未完成，请重试".to_string())?

}

/// 启动应用。桌面由 main.rs 调用，Android 由系统通过 JNI 直接进这里。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();

    // 单实例只对桌面有意义：Android 上一个应用本来就只有一个前台实例，
    // 这个插件在移动端也没有对应实现。
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder
        // 单实例必须第一个注册。开第二个时把已有窗口提上来，然后自己退出——
        // 否则两个进程各持一份内存模型，交替保存会把对方的改动静默覆盖掉。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }));

    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            #[cfg(any(target_os="android",target_os="ios"))]
            let base_dir = app.path().app_data_dir()?;
            #[cfg(target_os="android")]
            let base_dir = base_dir.join("files");
            #[cfg(not(any(target_os="android",target_os="ios")))]
            let base_dir = exe_dir();
    let (path, config_error) = resolve_vault_path(&base_dir);
    if let Some(e) = &config_error {
        eprintln!("配置错误：{}", e);
    }

    let state = AppState {
        action: Mutex::new(()),
        session: Mutex::new(0),
        recovering: Mutex::new(false),
        store: Mutex::new(Store::new()),
        path,
        selected: Mutex::new(String::new()),
        imported: Mutex::new(Vec::new()),
        using_default_pwd: Mutex::new(false),
        fails: Mutex::new(0),
        retry_until: Mutex::new(None),
        picking: std::sync::atomic::AtomicBool::new(false),
        import_request: std::sync::atomic::AtomicU64::new(0),
        last_activity: Mutex::new(Instant::now()),
        pending: Mutex::new(None),
        config_error,
        dark_theme: Mutex::new(true),
        theme_checks: Mutex::new(None),
    };

            app.manage(state);
            #[cfg(target_os="android")]
            app.handle().plugin(tauri::plugin::Builder::<tauri::Wry>::new("secback-native")
                .setup(|app, api| { let handle = api.register_android_plugin("com.secback.vault", "SecBackPlugin")?; let watcher_app = app.clone();
                    let channel = tauri::ipc::Channel::<Value>::new(move |_| {
                        if watcher_app.state::<AppState>().picking.load(std::sync::atomic::Ordering::SeqCst) { return Ok(()); }
                        let app = watcher_app.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            let st = app.state::<AppState>(); let _action=lock!(st.action);
                            if !st.picking.load(std::sync::atomic::Ordering::SeqCst) { do_lock(&app, &st); }
                        }); Ok(())
                    });
                    let watcher = handle.clone();
                    std::thread::spawn(move || { let _ = watcher.run_mobile_plugin::<Value>("watch",json!({"channel":channel})); });
                    app.manage(Native(handle)); Ok(()) }).build())?;
            spawn_idle_watchdog(app.handle().clone());
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            install_menu(app.handle(), false);

            // 拖放一个密库文件到窗口上就导入。Tauri 默认拦截系统拖放事件，
            // 走它自己的回调才能拿到绝对路径。
            if let Some(w) = app.get_webview_window("main") {
                let h = app.handle().clone();
                w.on_window_event(move |event| {
                    if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop {
                        paths, ..
                    }) = event
                    {
                        if let Some(p) = paths.first().cloned() {
                            let h = h.clone();
                            tauri::async_runtime::spawn_blocking(move || {
                            let st = h.state::<AppState>();
                            let _action = lock!(st.action);
                            start_import(&h, st.inner(), p);
                            });
                        }
                    }
                });
            }
            Ok(())
        });

    // 菜单事件只存在于桌面端（移动端没有 tauri::menu）。
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.on_menu_event(|app, event| {
            let app = app.clone();
            let event_id = event.id().as_ref().to_string();
            tauri::async_runtime::spawn_blocking(move || {
            let st = app.state::<AppState>();
            let _action = lock!(st.action);
            let app = &app;
            match event_id.as_str() {
                "new" => push(app, json!({ "t": "cmd", "name": "new" })),
                "delete" => push(app, json!({ "t": "cmd", "name": "delete" })),
                "import" => pick_and_import(app.clone()),
                "export" => pick_and_export(app.clone()),
                "changePwd" => push(app, json!({ "t": "cmd", "name": "changePwd" })),
                "setRecovery" => push(app, json!({ "t": "cmd", "name": "setRecovery" })),
                "theme-dark" => {
                    apply_theme(app, st.inner(), true);
                    push(app, json!({ "t": "cmd", "name": "theme-dark" }));
                }
                "theme-light" => {
                    apply_theme(app, st.inner(), false);
                    push(app, json!({ "t": "cmd", "name": "theme-light" }));
                }
                "lock" => do_lock(app, st.inner()),
                "about" => push(app, json!({ "t": "cmd", "name": "about" })),
                "quit" => app.exit(0),
                _ => {}
            }
            });
        });

    builder
        .invoke_handler(tauri::generate_handler![host_message])
        .run(tauri::generate_context!())
        .expect("SecBack 启动失败");
}

#[allow(dead_code)]
fn unused(_: MutexGuard<'_, Store>) {}

#[cfg(test)]
mod import_reader_tests {
    use super::*;
    #[test]
    fn rejects_oversized_and_failed_document_reads() {
        assert!(read_import_bytes(std::io::repeat(0)).is_err());
        struct Failed;
        impl Read for Failed { fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("provider unavailable"))
        }}
        assert!(read_import_bytes(Failed).is_err());
        assert_eq!(read_import_bytes(&b"snapshot"[..]).unwrap(), b"snapshot");
    }
}
