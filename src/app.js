// SecBack 前端。
//
// 所有数据都由 Tauri 宿主推送（snapshot / secret / toast），这里只负责渲染和把
// 用户操作翻译成命令发回去。快照只含 hasPass；显示详情或编辑时会临时取得明文，
// 保存、关闭和锁定时清理引用。JavaScript 无法保证 GC 内存物理归零。

// 传输层：同一套 {t:...} 协议，两种承载。
//   Tauri    : invoke("host_message") 发、listen("host") 收
//   WebView2 : postMessage / message 事件
// 分开写是为了同一份前端既能跑桌面 Tauri，也能直接跑在手机的 Tauri 里。
const tauriApi = window.__TAURI__;
const webview2 = window.chrome && window.chrome.webview;

function send(obj) {
  const s = JSON.stringify(obj);
  if (tauriApi) return tauriApi.core.invoke("host_message", { json: s }).catch(error => {
    const text = String(error || "请求失败，请重试");
    if (obj.t === "login") { loginPending = false; $("login-go").disabled = false; setHint(text,"error"); }
    if (["create","save"].includes(obj.t)) { savingRequest=null; $("modal-save").disabled=false; $("overlay").querySelector(".modal-body").inert=false; }
    if (obj.t === "import_commit") { $("imp-ok").disabled=false; $("imp-tree").inert=false; }
    toast("err", text);
  });
  else if (webview2) webview2.postMessage(s);
}

function onHost(cb) {
  if (tauriApi) {
    // Tauri 的 listen 是异步的：必须等它注册完再发 ready，
    // 否则宿主的首条消息（bootstrap）会打在监听器就绪之前，直接丢掉。
    return tauriApi.event.listen("host", e => cb(e.payload));
  }
  if (webview2) {
    webview2.addEventListener("message", e => cb(e.data));
  }
  return Promise.resolve();
}

let S = {
  projects: [],
  selected: "",
  canUp: false,
  canDown: false,
  defaultPwd: false
};

let query = "";
let secrets = {};

// 是否"真的要显示详情"。
// 不能只看 S.selected 有没有值——拖拽排序后 renderList 会重建、
// renderDetail 看到 selected 还在就把详情又显示出来了（K90 上就是这个 bug）。
let wantDetail = false;
let editingId = null;      // null 表示新增
let editSlotCount = 1;     // 编辑弹层里显示几个账户槽（1~3，密库格式固定 3 个）
let expandedSlot = 0;      // 当前展开的是哪个槽，其余的收起来
let impData = null;        // 导入预览的临时状态

// 编辑弹层专用的 reveal 请求指纹。它和详情页的"显示"走同一条消息，
// 但拿到后只填输入框、不写进 secrets 缓存，少留一份明文在内存里。
const editReveals = new Set();
const detailReveals = new Map();
let detailRequest = 0;
let editorSession = 0;
let savingRequest = null;
let confirmedDeleteId = null;
let firstRun = false, loginPending = false, loginDeadline = 0, loginTick = null;
let editorDirty = false, importRevision = 0;
let backButtonRegistration = null;

const $ = id => document.getElementById(id);

// 共用线性图标；图标不读屏，按钮保留中文名称。
function icon(name) {
  const paths = {
    copy: '<rect x="8" y="8" width="12" height="13" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/>',
    eye: '<path class="eye-open" d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle class="eye-open" cx="12" cy="12" r="3"/><path class="eye-closed" d="m3 3 18 18M10.6 5.1A12 12 0 0 1 12 5c6.5 0 10 7 10 7a18 18 0 0 1-3.1 3.9M6.1 6.1A21 21 0 0 0 2 12s3.5 7 10 7a12 12 0 0 0 5.9-1.9M9.9 9.9a3 3 0 0 0 4.2 4.2"/>',
    edit: '<path d="m16 3 5 5M4 15 16 3a2 2 0 0 1 3 0l2 2a2 2 0 0 1 0 3L9 20l-6 1Z"/>',
    trash: '<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7"/>',
    save: '<path d="M20 21H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h13l4 4v13a1 1 0 0 1-1 1Z"/><path d="M7 3v6h10V3M7 21v-8h10v8"/>',
    cancel: '<path d="m6 6 12 12M18 6 6 18"/>',
    import: '<path d="M12 3v12m-4-4 4 4 4-4M4 16v5h16v-5"/>',
    file: '<path d="M14 3H5v18h14V8Zm0 0v5h5M8 12h8M8 16h5"/>',
    palette: '<path d="M12 3a9 9 0 1 0 0 18h1a2 2 0 0 0 1-3.7 1.6 1.6 0 0 1 1-2.8h2a4 4 0 0 0 4-4A8.5 8.5 0 0 0 12 3Z"/><circle cx="7.5" cy="11" r=".5"/><circle cx="10" cy="7.5" r=".5"/><circle cx="15" cy="8" r=".5"/>',
    help: '<circle cx="12" cy="12" r="9"/><path d="M9.5 9a2.5 2.5 0 1 1 4 2c-1 .7-1.5 1-1.5 2.5M12 17h.01"/>',
    plus: '<path d="M12 4v16M4 12h16"/>',
    export: '<path d="M12 16V3m-4 4 4-4 4 4M4 16v5h16v-5"/>',
    lock: '<rect x="5" y="10" width="14" height="11" rx="3"/><path d="M8 10V7a4 4 0 0 1 8 0v3M12 14v3"/>',
    key: '<circle cx="8" cy="9" r="5"/><path d="m12 13 8 8m-4-4 3-3m-6 0 3-3"/>',
    shield: '<path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6ZM9 12l2 2 4-4"/>',
    moon: '<path d="M20.5 13A8.5 8.5 0 0 1 11 3.5 8.5 8.5 0 1 0 20.5 13Z"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5 19 19M5 19l1.5-1.5M17.5 6.5 19 5"/>',
    enter: '<path d="M14 3h5a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-5M3 12h12m-4-4 4 4-4 4"/>'
  };
  return `<svg class="button-icon" viewBox="0 0 24 24" aria-hidden="true">${paths[name]}</svg>`;
}

function setActionLabel(button, text, symbol) {
  button.classList.add("with-icon");
  button.innerHTML = icon(symbol) + `<span>${esc(text)}</span>`;
}

for (const [id, symbol] of Object.entries({
  "imp-ok":"import", "btn-edit":"edit", "btn-delete":"trash", "modal-save":"save", "confirm-ok":"trash",
  "modal-cancel":"cancel", "confirm-cancel":"cancel", "ipwd-cancel":"cancel",
  "imp-cancel":"cancel", "pwd-cancel":"cancel", "recpwd-cancel":"cancel",
  "recover-cancel":"cancel", "recnew-cancel":"cancel", "sheet-cancel":"cancel"
})) setActionLabel($(id), $(id).textContent, symbol);
setActionLabel($("login-go"), "进入", "enter");
$("login-visible").innerHTML = icon("eye");

function setLoginPasswordVisible(show) {
  $("login-pwd").type = $("login-confirm").type = show ? "text" : "password";
  $("login-visible").setAttribute("aria-label", show ? "隐藏主密码" : "显示主密码");
  $("login-visible").setAttribute("title", show ? "隐藏主密码" : "显示主密码");
  $("login-visible").setAttribute("aria-pressed", String(show));
}

// ------------------------------------------------------------------ 主题
//
// 默认深色。用户在菜单里选过就以 localStorage 为准。
// HTML 标签上已经写死 data-theme="dark"，所以首帧就是深的，不会先闪一下白。

function applyTheme(name, persist = true) {
  document.documentElement.dataset.theme = name === "light" ? "light" : "dark";
  if (persist) {
    try { localStorage.setItem("secback.theme", name); } catch (_) {}
  }
  // 让宿主把窗口标题栏也调成一致，否则深色界面顶个白标题栏很割裂
  send({ t: "theme_sync", dark: name !== "light" });
  syncDesktopMenu();
}

function currentTheme() {
  try { return localStorage.getItem("secback.theme") || "dark"; } catch (_) { return "dark"; }
}

// ------------------------------------------------------------------ 登录视图

function setHint(text, kind) {
  const el = $("login-hint");
  el.textContent = text || "";
  el.className = "login-hint" + (kind ? " " + kind : "");
}

function showLogin(hint, kind) {
  $("app").hidden = true;
  $("login").hidden = false;
  closeDesktopMenu();
  syncDesktopMenu();
  $("login-pwd").value = "";
  loginPending = false;
  $("login-go").disabled = Date.now() < loginDeadline;
  $("login-confirm").hidden = !firstRun;
  setActionLabel($("login-go"), firstRun ? "创建密库" : "进入", "enter");
  setLoginPasswordVisible(false);
  setHint(hint || "", kind);
  // 这里**不再**自动 focus。
  // 自动聚焦会让系统键盘在进 App 的瞬间就弹出来，挡住半屏，
  // 而且用户还没打算输入。改成用户点输入框时再弹（浏览器原生行为）。
}

function showMain() {
  $("login").hidden = true;
  $("app").hidden = false;
  syncDesktopMenu();
  if(S.defaultPwd)toast("warn","正在使用默认主密码，请在「更多」中修改");
}

function doLogin() {
  if (loginPending || Date.now() < loginDeadline || $("login-pwd").disabled) return;
  const pwd = $("login-pwd").value;
  if (!pwd) {
    $("login-pwd").focus();
    return;
  }
  const confirm = $("login-confirm").value;
  if (firstRun && ([...pwd].length < 6 || pwd !== confirm)) { setHint("请填写两次一致的主密码（至少 6 字符）","error"); return; }
  loginPending = true;
  $("login-go").disabled = true;
  setHint(firstRun ? "正在创建…" : "正在解锁…");
  send({ t: "login", pwd, confirm });
}

$("login-go").addEventListener("click", doLogin);
$("login-confirm").addEventListener("keydown", e => { if(e.key==="Enter") doLogin(); });
$("login-visible").addEventListener("click", () => {
 setLoginPasswordVisible($("login-pwd").type === "password");
});
$("login-pwd").addEventListener("keydown", e => {
  if (e.key === "Enter") doLogin();
});

function esc(s) {
  return String(s == null ? "" : s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function toast(kind, text) {
  const el = document.createElement("div");
  el.className = "toast " + (kind || "info");
  el.textContent = text;
  $("toasts").appendChild(el);
  setTimeout(() => {
    el.style.transition = "opacity .2s";
    el.style.opacity = "0";
    setTimeout(() => el.remove(), 220);
  }, 2400);
}

// ------------------------------------------------------------------ 渲染

function projectById(id) {
  return S.projects.find(p => p.id === id) || null;
}

function visibleProjects() {
  if (!query) return S.projects;
  const q = query.toLowerCase();
  return S.projects.filter(p => {
    if (p.name.toLowerCase().includes(q)) return true;
    if (p.remark.toLowerCase().includes(q)) return true;
    return p.slots.some(s => !s.empty && s.user.toLowerCase().includes(q));
  });
}

function initial(name) {
  const t = (name || "").trim();
  if (!t) return "?";
  // 中文取第一个字；英文取首字母大写
  return t[0].toUpperCase();
}

function slotCount(p) {
  return p.slots.filter(s => !s.empty).length;
}

function renderList() {
  const list = $("list");
  const items = visibleProjects();

  if (!S.projects.length) {
    list.innerHTML = "";
    $("list-empty").hidden = false;
    $("list-empty").querySelector(".empty-title").textContent = "还没有项目";
    $("list-empty").querySelector(".empty-sub").textContent = "点「新增」建立第一个项目";
    $("count").textContent = "";
    return;
  }
  $("list-empty").hidden = items.length > 0;
  $("list-empty").querySelector(".empty-title").textContent = "没有匹配的项目";
  $("list-empty").querySelector(".empty-sub").textContent = "请修改或清空搜索条件";

  list.innerHTML = items.map(p => {
    const cls = ["item"];
    if (p.id === S.selected) cls.push("active");
    if (p.imported) cls.push("imported");

    const sub = p.remark
      ? p.remark
      : (slotCount(p) ? slotCount(p) + " 套账户" : "暂无账户");

    return `<div class="${cls.join(" ")}" data-id="${p.id}" role="option" tabindex="${p.id===S.selected?0:-1}" aria-selected="${p.id===S.selected}">
      <div class="avatar">${esc(initial(p.name))}</div>
      <div class="item-main">
        <div class="item-name">${esc(p.name)}</div>
        <div class="item-sub">${esc(sub)}</div>
      </div>
    </div>`;
  }).join("");

  const total = S.projects.length;
  $("count").textContent = query
    ? `${items.length} / ${total} 个项目`
    : `${total} 个项目`;
}

function secretKey(id, slot) {
  return id + ":" + slot;
}

function renderDetail() {
  const p = projectById(S.selected);

  $("placeholder").hidden = !!p;
  $("pane").hidden = !p;

  // 窄屏下切到详情视图。必须是"确实想看详情"才切：
  // 只看 p 有没有值的话，拖拽排序触发的重渲染会把详情又翻出来。
  const showDetail = !!p && wantDetail;
  document.body.classList.toggle("showing-detail", showDetail);

  if (!p) {
    $("slots").replaceChildren();
    $("p-name").textContent = "";
    $("p-remark").textContent = "";
    return;
  }

  $("p-name").textContent = p.name;
  $("p-remark").textContent = p.remark || "";

  $("btn-up").disabled = !S.canUp;
  $("btn-down").disabled = !S.canDown;

  const slots = p.slots.map((s, i) => {
    if (s.empty) {
      return `<div class="slot empty-slot">
        <div class="slot-head">
          <span class="slot-index">账户 ${i + 1}</span>
          <span class="slot-tag">空</span>
        </div>
      </div>`;
    }

    const key = secretKey(p.id, i);
    const revealed = Object.prototype.hasOwnProperty.call(secrets, key);
    const passText = revealed ? secrets[key] : (s.hasPass ? "••••••••" : "（无密码）");

    return `<div class="slot" data-slot="${i}">
      <div class="slot-head">
        <span class="slot-index">账户 ${i + 1}</span>
      </div>

      <div class="row">
        <span class="row-label">账号</span>
        <span class="row-value">${esc(s.user) || '<span class="row-value dim">（空）</span>'}</span>
        <button class="mini icon-action" data-act="copy-user" data-slot="${i}" aria-label="复制账户 ${i + 1} 的账号" title="复制账号">${icon("copy")}</button>
      </div>

      <div class="row">
        <span class="row-label">密码</span>
        <span class="row-value" id="pass-${i}">${esc(passText)}</span>
        <button class="mini icon-action eye-toggle" data-act="toggle-pass" data-slot="${i}" aria-label="${revealed ? "隐藏" : "显示"}账户 ${i + 1} 的密码" title="${revealed ? "隐藏" : "显示"}密码" aria-pressed="${revealed}">${icon("eye")}</button>
        <button class="mini icon-action" data-act="copy-pass" data-slot="${i}" aria-label="复制账户 ${i + 1} 的密码" title="复制密码">${icon("copy")}</button>
      </div>

      ${s.remark ? `<div class="row">
        <span class="row-label">备注</span>
        <span class="row-value dim">${esc(s.remark)}</span>
      </div>` : ""}
    </div>`;
  }).join("");

  $("slots").innerHTML = slots;

  const upd = p.imported ? "　·　本次导入" : "";
  $("p-meta").textContent = `共 ${slotCount(p)} 套账户${upd}`;
}

function render() {
  renderList();
  renderDetail();
  $("btn-up").disabled = !S.canUp;
  $("btn-down").disabled = !S.canDown;
}

// ------------------------------------------------------------------ 接收宿主消息

function applySnapshot(msg) {
  S.projects = msg.projects || [];
  S.selected = msg.selected || "";
  S.canUp = !!msg.canUp;
  S.canDown = !!msg.canDown;
  S.defaultPwd = !!msg.defaultPwd;
  $("btn-more").classList.toggle("warn",S.defaultPwd);
  $("btn-more").title = S.defaultPwd ? "正在使用默认主密码，请到更多中修改" : "更多设置";

  secrets = {};
  detailReveals.clear();

  render();
}

function onSecret(msg) {
  if (typeof msg.request === "string" && msg.request.startsWith("detail:")) {
    const key = secretKey(msg.id, msg.slot);
    if (detailReveals.get(key) !== msg.request || $("app").hidden || msg.id !== S.selected) return;
    detailReveals.delete(key);
    secrets[key] = msg.pass;
    renderDetail();
    return;
  }
  if (msg.request != null) {
    if (!editReveals.delete(msg.request)) return;
    if (editingId === msg.id && !$('overlay').hidden) {
      const box = $('m-slots').querySelector('.slot-edit[data-source="' + msg.slot + '"]');
      const inp = box?.querySelector('[data-f="pass"]');
      if (inp && inp.dataset.edited !== '1') {
        inp.value = msg.pass;
        inp.placeholder = '';
      }
    }
    return;
  }
  if ($('app').hidden || msg.id !== S.selected) return;

  secrets[secretKey(msg.id, msg.slot)] = msg.pass;
  if (msg.id === S.selected) renderDetail();
}

onHost(raw => {
    // Tauri 的 payload 是字符串，WebView2 的也可能是；统一解析一次
    let m = raw;
    if (typeof m === "string") {
      try { m = JSON.parse(m); } catch (_) { return; }
    }
    if (!m || !m.t) return;

    if (m.t === "bootstrap") {
      firstRun = !!m.firstRun;
      document.documentElement.dataset.platform = m.platform || "unknown";
      if (m.platform === "android" && !backButtonRegistration) {
        backButtonRegistration = registerBackButton();
        backButtonRegistration.catch(() => toast("err", "系统返回键初始化失败，请重新打开应用"));
      }
      // 配置文件写错：明确挡住登录。放进去就等于在错误的位置新建空库，
      // 用户回头会以为"数据丢了"。
      if (m.configError) {
        showLogin(m.configError, "error");
        $("login-pwd").disabled = true;
        $("login-go").disabled = true;
        return;
      }
      // 首次运行要把密库位置写出来——这个程序最容易出的事就是"密库到底在哪儿"
      const hint = m.firstRun
        ? "没有找到密库，将新建一份空的。\n位置：\n" + m.vaultPath
        : "";
      showLogin(hint);
      return;
    }

    if (m.t === "loggedIn") {
      firstRun=false; loginPending=false; loginDeadline=0; clearInterval(loginTick);
      document.querySelectorAll('#login input, #recover input, #recnew input').forEach(el => el.value = "");
      showMain();
      return;
    }

    if (m.t === "loginError") {
      loginPending=false; clearInterval(loginTick);
      loginDeadline=Date.now()+(m.retryAfter||0);
      const update=()=>{
        const left=Math.ceil((loginDeadline-Date.now())/1000);
        $("login-go").disabled=left>0;
        setHint(left>0 ? "密码连续错误，请等待 "+left+" 秒" : (m.text||"密码不正确"),"error");
        if(left<=0)clearInterval(loginTick);
      };
      update(); if(m.retryAfter>0)loginTick=setInterval(update,250);
      $("login-pwd").value="";
      return;
    }

    if (m.t === "loggedOut") {
      savingRequest = null;
      closeEditor();
      document.querySelectorAll(".overlay").forEach(el => el.hidden = true);
      document.querySelectorAll("input, textarea").forEach(el => el.value = "");
      $("reccode-text").textContent = "";
      query = "";
      confirmedDeleteId = null;
      wantDetail = false;
      S = { projects: [], selected: "", canUp: false, canDown: false, defaultPwd: false };
      secrets = {};
      editReveals.clear();
      detailReveals.clear();
      $("imp-tree").replaceChildren();
      $("imp-file").textContent = "";
      $("imp-sum").textContent = "";
      $("ipwd-file").textContent = "";
      $("toasts").replaceChildren();
      $("backup-status").textContent = "";

      // 锁定时必须把弹层收掉，并把里面的内容清空。
      // 否则编辑框会盖在登录页上，而里面还留着刚取出来的明文密码。
      $("overlay").hidden = true;
      $("confirm").hidden = true;
      $("imp").hidden = true;
      $("ipwd").hidden = true;
      $("pwd").hidden = true;
      $("recpwd").hidden = true;
      $("reccode").hidden = true;
      $("recover").hidden = true;
      $("recover-input").value = "";
      $("recnew").hidden = true;
      $("recpwd-input").value = "";
      $("recover-input").value = "";
      $("rec-confirm").value = "";
      $("recnew-1").value = "";
      $("recnew-2").value = "";
      recCodeText = "";
      // 退出登录后回到列表视图，别让下次登录直接停在详情页
      document.body.classList.remove("showing-detail");
      $("m-slots").innerHTML = "";
      $("m-name").value = "";
      $("m-remark").value = "";
      $("confirm-name").textContent = "";
      $("ipwd-input").value = "";
      $("pwd-old").value = "";
      $("pwd-new").value = "";
      $("pwd-new2").value = "";
      impData = null;
      editingId = null;

      render();
      showLogin("已锁定，请重新输入主密码。");
      return;
    }

    if (m.t === "exportResult") {
      $("backup-status").textContent = "本次备份：" + new Date().toLocaleString("zh-CN") + "\n" + m.destination;
      return;
    }
    if (m.t === "saveResult") {
      if (m.request !== savingRequest) return;
      savingRequest = null;
      $("modal-save").disabled = false;
      $("overlay").querySelector(".modal-body").inert = false;
      if (m.ok) closeEditor();
      return;
    }
    if (m.t === "rekeyResult") {
      if (m.ok) $("pwd-cancel").click();
      return;
    }
    if (m.t === "selection") {
      S.selected=m.selected; S.canUp=!!m.canUp; S.canDown=!!m.canDown;
      document.querySelectorAll("#list .item").forEach(el=>{const on=el.dataset.id===S.selected;el.classList.toggle("active",on);el.setAttribute("aria-selected",String(on));el.tabIndex=on?0:-1;});
      secrets={};detailReveals.clear();renderDetail();
      $("btn-up").disabled=!S.canUp;$("btn-down").disabled=!S.canDown;return;
    }
    if (m.t === "importPlan") { if(impData && m.request===importRevision){impData.summary=m.summary;renderImportSummary();}return; }
    if (m.t === "importResult") {
      $("imp-ok").disabled=false; $("imp-tree").inert=false;
      if(m.ok){$("imp").hidden=true;$("imp-tree").replaceChildren();impData=null;}else{$("imp-sum").textContent="导入失败："+(m.text||"请重试");}return;
    }
    if (m.t === "snapshot") {
      applySnapshot(m);
      return;
    }
    if (m.t === "secret") {
      onSecret(m);
      return;
    }
    if (m.t === "toast") {
      toast(m.kind, m.text);
      return;
    }
    if (m.t === "importNeedPassword") {
      $("ipwd-file").textContent = m.fileName || "";
      $("ipwd-input").value = "";
      $("ipwd-err").textContent = "";
      $("ipwd").hidden = false;
      setTimeout(() => $("ipwd-input").focus(), 30);
      return;
    }

    if (m.t === "importError") {
      $("ipwd-err").textContent = m.text || "读取失败";
      $("ipwd-input").value = "";
      $("ipwd-input").focus();
      return;
    }

    if (m.t === "importPreview") {
      $("ipwd").hidden = true;
      $("ipwd-input").value = "";
      showImportPreview(m);
      return;
    }

    if (m.t === "recoveryCode") {
      showRecoveryCode(m.code);
      return;
    }

    if (m.t === "recoverOk") {
      // 解开了，但还没设新密码——必须先设好才能进，否则改了数据一保存，
      // 库里存的仍是那把已经忘掉的口令。
      $("recover").hidden = true;
      $("recnew").hidden = false;
      $("recnew-1").value = "";
      $("recnew-2").value = "";
      $("recnew-err").textContent = "";
      setTimeout(() => $("recnew-1").focus(), 30);
      return;
    }

    if (m.t === "recoveryError" || m.t === "recoverError") {
      showRecError(m.text || "操作失败");
      return;
    }

    if (m.t === "cmd") {
      if (m.name === "new") openEditor(null);
      if (m.name === "delete") askDelete();
      if (m.name === "about") send({ t: "about" });
      if (m.name === "changePwd") openChangePwd();
      if (m.name === "theme-dark") applyTheme("dark");
      if (m.name === "theme-light") applyTheme("light");
      if (m.name === "setRecovery") openSetRecovery();
      return;
    }

}).then(() => {
  // 恢复上次选的外观，并让宿主同步窗口标题栏颜色。
  // 必须在监听器就绪之后：applyTheme 里会发消息。
  applyTheme(currentTheme(), false);
  // 然后才打招呼，别让宿主的第一条消息白白丢掉
  send({ t: "ready" });
});

// ------------------------------------------------------------------ 列表交互

// 选中某个项目并进入详情。
// 抽成函数是因为入口有两个：桌面用 click，触摸屏用 touchend（见 setupDragSort）。
function selectProject(id) {
  if (id === S.selected) {
    // 从详情按返回回到列表后，再点同一个项目要能重新进去。
    wantDetail = true;
    document.body.classList.add("showing-detail");
    return;
  }
  S.selected = id;
  wantDetail = true;
  secrets = {};
  send({ t: "select", id });
  document.querySelectorAll("#list .item").forEach(el=>{const on=el.dataset.id===id;el.classList.toggle("active",on);el.setAttribute("aria-selected",String(on));el.tabIndex=on?0:-1;});
  renderDetail();

}

$("list").addEventListener("click", e => {
  // 触摸设备上点击由 touchend 负责（见 setupDragSort 里的 finish）。
  // 这里只服务桌面鼠标：最近 800ms 内有 touchend 就说明是手机，跳过。
  if (Date.now() - Number(document.body.dataset.lastTouch || 0) < 800) return;

  const item = e.target.closest(".item");
  if (!item) return;
  selectProject(item.dataset.id);
});

// 窄屏才可见的返回按钮：退回列表，但不清选中——
// 这样再点同一个项目还能进来。
$("pane-back").addEventListener("click", () => {
  handleBack();
});

// ------------------------------------------------------------------ 详情交互

$("detail").addEventListener("click", e => {
  const btn = e.target.closest("button[data-act]");
  if (!btn || !S.selected) return;

  const slot = parseInt(btn.dataset.slot, 10);
  const act = btn.dataset.act;
  const key = secretKey(S.selected, slot);

  if (act === "copy-user") {
    send({ t: "copy", id: S.selected, slot, field: "user" });
    return;
  }
  if (act === "copy-pass") {
    // 没显示过也能复制：宿主直接取，不经过前端
    send({ t: "copy", id: S.selected, slot, field: "pass" });
    return;
  }
  if (act === "toggle-pass") {
    if (Object.prototype.hasOwnProperty.call(secrets, key)) {
      delete secrets[key];
      detailReveals.delete(key);
      renderDetail();
    } else {
      // 先占位，收到 secret 再重绘
      secrets[key] = "…";
      renderDetail();
      const request = "detail:" + ++detailRequest;
      detailReveals.set(key, request);
      send({ t: "reveal", id: S.selected, slot, request });
    }
  }
});

$("btn-edit").addEventListener("click", () => {
  const p = projectById(S.selected);
  if (p) openEditor(p);
});

// 删除不可逆，先弹确认。确认框里带上项目名，避免删错。
// 按工具栏按钮和按菜单走的是同一个入口。
function askDelete() {
  if (!$("confirm").hidden || !$("overlay").hidden || $("app").hidden) return;
  const p = projectById(S.selected);
  if (!p) return;
  confirmedDeleteId = p.id;
  $("confirm-name").textContent = p.name;
  $("confirm").hidden = false;
  // 焦点给"取消"：用户按了删除后手可能还在回车上，不能让他一按就把项目删了
  setTimeout(() => $("confirm-cancel").focus(), 30);
}

$("btn-delete").addEventListener("click", askDelete);

$("confirm-cancel").addEventListener("click", () => {
  $("confirm").hidden = true;
});

$("confirm-ok").addEventListener("click", () => {
  $("confirm").hidden = true;
  if (confirmedDeleteId) send({ t: "delete", id: confirmedDeleteId });
  confirmedDeleteId = null;
});

// 点遮罩空白处等同取消；点到弹层本体不算
$("confirm").addEventListener("click", e => {
  if (e.target.id === "confirm") $("confirm").hidden = true;
});

// ------------------------------------------------------------------ 工具栏

function move(dir) {
  if (!S.selected) return;
  send({ t: dir < 0 ? "up" : "down" });
}

// 新增：传 null 就是"还没有项目"的空白编辑弹层
$("btn-new").addEventListener("click", () => openEditor(null));
$("btn-up").addEventListener("click", () => move(-1));
$("btn-down").addEventListener("click", () => move(1));
$("btn-import").addEventListener("click", () => send({ t: "import" }));
$("btn-lock").addEventListener("click", () => send({ t: "lock" }));

$("q").addEventListener("input", e => {
  query = e.target.value.trim();
  renderList();
});

// Ctrl+N / Ctrl+Up / Ctrl+Down / Del 与菜单保持同一套快捷键
document.addEventListener("keydown", e => {
  if ($("overlay").hidden === false) return;
  if ($("confirm").hidden === false) return;
  if ($("imp").hidden === false) return;
  if ($("ipwd").hidden === false) return;
  if ($("pwd").hidden === false) return;
  if ($("recpwd").hidden === false) return;
  if ($("reccode").hidden === false) return;
  if ($("recover").hidden === false) return;
  if ($("recnew").hidden === false) return;

  if (e.defaultPrevented || $("app").hidden) return;
  if (e.ctrlKey && e.key.toLowerCase() === "n") {
    e.preventDefault();
    openEditor(null);
    return;
  }
  if (e.ctrlKey && e.key === "ArrowUp") {
    e.preventDefault();
    move(-1);
    return;
  }
  if (e.ctrlKey && e.key === "ArrowDown") {
    e.preventDefault();
    move(1);
    return;
  }
  if (e.key === "Delete" && S.selected && !document.activeElement.closest("input,textarea,[contenteditable=true]")) {
    e.preventDefault();
    askDelete();
  }
});

// ------------------------------------------------------------------ 编辑弹层

function setEditorPasswordVisible(input, show) {
  input.type = show ? "text" : "password";
  const button = input.parentElement.querySelector('[data-visibility]');
  button.setAttribute("aria-label", show ? "隐藏密码" : "显示密码");
  button.setAttribute("aria-pressed", String(show));
}

function slotEditHtml(i, s, expanded, removable) {
  const empty = !s || s.empty;
  const hasPass = !empty && s.hasPass;
  // 收起时用一行摘要代替三个输入框，避免"九个框堆一起"的压迫感
  const summary = empty ? "" : (s.user ? esc(s.user) : "未填账号");

  // 编辑密码单独读取并默认遮盖，用户可主动切换显示（见 openEditor）。
  // 框里先空着并提示"读取中…"，拿到之前用户要是已经动手输入，就不再覆盖。
  return `<div class="slot-edit${expanded ? " expanded" : ""}" data-slot="${i}" data-source="${s ? i : -1}">
    <div class="slot-edit-head">
      <button class="slot-toggle" type="button" data-toggle="${i}">
        <svg class="chev" viewBox="0 0 16 16" aria-hidden="true"><path d="M6 4l4 4-4 4"/></svg>
        <span class="slot-name">账户 ${i + 1}</span>
        ${hasPass ? '<em class="has-pass">已设置密码</em>' : ""}
        ${!expanded && summary ? `<em class="slot-summary">${summary}</em>` : ""}
      </button>
      <button class="clear" data-clear="${i}">清空</button>
      ${removable ? `<button class="slot-remove" data-remove="${i}" aria-label="删除这个账户">✕</button>` : ""}
    </div>
    <div class="grid"${expanded ? "" : " hidden"}>
      <label class="ifield">
        <svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="5.2" r="2.6"/><path d="M3 13.4c0-2.6 2.2-4.2 5-4.2s5 1.6 5 4.2"/></svg>
        <input data-f="user" aria-label="账号" placeholder="账号" value="${esc(empty ? "" : s.user)}">
      </label>
      <div class="ifield password-field">
        <svg viewBox="0 0 16 16" aria-hidden="true"><rect x="3.2" y="7" width="9.6" height="6.2" rx="1.6"/><path d="M5.6 7V5.2a2.4 2.4 0 0 1 4.8 0V7"/></svg>
        <input data-f="pass" aria-label="密码" type="password" autocomplete="off" spellcheck="false"
               placeholder="${hasPass ? "读取中…" : "密码"}"
               value="" data-haspass="${hasPass ? "1" : "0"}">
        <button type="button" class="password-visibility" data-visibility aria-label="显示密码" aria-pressed="false">
          ${icon("eye")}
        </button>
      </div>
      <label class="ifield full">
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 3h9v6.2l-3.3 3.3H3.5z"/><path d="M12.5 9.2H9.2v3.3"/></svg>
        <input data-f="remark" aria-label="账户备注" placeholder="备注" value="${esc(empty ? "" : s.remark)}">
      </label>
    </div>
  </div>`;
}

// 按当前计数重画槽列表。只在"打开弹层"时调用一次——
// 增删槽走的是 DOM 增删（见下面的事件），不重画，否则已填的内容会丢。
function renderSlotsEdit(p) {
  let html = "";
  for (let i = 0; i < editSlotCount; i++) {
    html += slotEditHtml(i, p ? p.slots[i] : null, i === expandedSlot, editSlotCount > 1);
  }
  if (editSlotCount < 3) {
    html += `<button class="add-slot" type="button" data-add="1">
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3.5v9M3.5 8h9"/></svg>
      添加账户
    </button>`;
  }
  $("m-slots").innerHTML = html;
}

function openEditor(p) {
  if (savingRequest || $("app").hidden) return;
  editorDirty=false;
  editorSession++;
  editReveals.clear();
  savingRequest = null;
  $("modal-save").disabled = false;
  $("overlay").querySelector(".modal-body").inert = false;
  editingId = p ? p.id : null;
  $("modal-title").textContent = p ? "编辑项目" : "新增项目";
  $("m-name").value = p ? p.name : "";
  $("m-remark").value = p ? p.remark : "";
  // 只显示实际有内容的槽（至少 1 个），不再固定摆三个空壳
  const filled = p ? p.slots.reduce((last, x, i) => x && !x.empty ? i + 1 : last, 0) : 0;
  editSlotCount = Math.max(1, Math.min(3, filled));
  expandedSlot = 0;
  renderSlotsEdit(p);

  // 已有密码的槽：按需取一次明文，填进输入框。
  // 走和详情页"显示"同一条消息，但记在 editReveals 里，收到后只填框、不进缓存。
  if (p) {
    for (let i = 0; i < 3; ++i) {
      if (!p.slots[i].empty && p.slots[i].hasPass) {
        const request = "edit:" + editorSession + ":" + i;
        editReveals.add(request);
        send({ t: "reveal", id: p.id, slot: i, request });
      }
    }
  }

  $("overlay").hidden = false;
  const modal = $("overlay").querySelector(".modal");
  // 显示后再量标题栏，让吸顶操作栏紧贴下方。
  modal.style.setProperty("--head-h", modal.querySelector(".modal-head").offsetHeight + "px");
  setTimeout(() => $("m-name").focus(), 30);
}

function requestCloseEditor() {
  if(savingRequest)return;
  if(editorDirty){$("discard").hidden=false;return;}
  closeEditor();
}
$("discard-cancel").addEventListener("click",()=>{$("discard").hidden=true;});
$("discard-ok").addEventListener("click",()=>{$("discard").hidden=true;closeEditor();});
$("overlay").addEventListener("input",()=>editorDirty=true);

function closeEditor() {
  if (savingRequest) return;
  editorSession++;
  editReveals.clear();
  $("m-name").value = "";
  $("m-remark").value = "";
  $("overlay").querySelector(".modal-body").inert = false;
  $("overlay").hidden = true;
  // 框里现在是明文密码，关掉就清干净，别留在 DOM 里
  $("m-slots").innerHTML = "";
  editingId = null;
}

$("modal-x").addEventListener("click", requestCloseEditor);
$("modal-cancel").addEventListener("click", requestCloseEditor);

$("overlay").addEventListener("click", e => {
  if (e.target === $("overlay")) requestCloseEditor();
});

$("m-slots").addEventListener("click", e => {
  const visibility=e.target.closest("[data-visibility]");
  if (visibility) {
    const input = visibility.parentElement.querySelector('[data-f="pass"]');
    setEditorPasswordVisible(input, input.type === "password");
    return;
  }
  if(e.target.closest("[data-clear],[data-add],[data-remove]"))editorDirty=true;
  // 清空
  const clearBtn = e.target.closest("button[data-clear]");
  if (clearBtn) {
    const box = clearBtn.closest(".slot-edit");
    box.querySelectorAll("input").forEach(inp => {
      inp.value = "";
      inp.dataset.mask = "0";
      inp.dataset.edited = "1";
      inp.placeholder = inp.dataset.f === "pass" ? "密码" : inp.placeholder;
    });
    return;
  }

  // 展开 / 收起：同时只展开一个，其余自动收起
  const toggle = e.target.closest("button[data-toggle]");
  if (toggle) {
    const idx = Number(toggle.dataset.toggle);
    if (expandedSlot === idx && document.querySelector(".slot-edit.expanded")) {
      // 已经展开就允许收起（全部收起也合法，方便快速扫一眼）
      expandedSlot = -1;
    } else {
      expandedSlot = idx;
    }
    syncSlotExpansion();
    return;
  }

  // 添加账户
  if (e.target.closest("[data-add]")) {
    if (editSlotCount >= 3) return;
    const idx = editSlotCount;
    editSlotCount++;
    expandedSlot = idx;
    const addBtn = $("m-slots").querySelector("[data-add]");
    const holder = document.createElement("div");
    holder.innerHTML = slotEditHtml(idx, null, true, true);
    const node = holder.firstElementChild;
    addBtn.parentNode.insertBefore(node, addBtn);
    syncSlotExpansion();
    if (editSlotCount >= 3) addBtn.remove();
    // 新槽自动获得焦点，省一次点击
    setTimeout(() => node.querySelector('[data-f="user"]').focus(), 30);
    return;
  }

  // 删除对应节点，保留其余账户的原始槽位和密码编辑状态。
  const rm = e.target.closest("button[data-remove]");
  if (rm) {
    if (editSlotCount <= 1) return;
    const boxes = [...$("m-slots").querySelectorAll(".slot-edit")];
    const idx = Number(rm.dataset.remove);
    boxes[idx].remove();
    editSlotCount--;
    if (expandedSlot >= idx) expandedSlot = Math.max(0, expandedSlot - 1);
    // 槽位下标要重排，否则 data-slot 和实际顺序对不上
    [...$("m-slots").querySelectorAll(".slot-edit")].forEach((box, i) => {
      box.dataset.slot = String(i);
      box.querySelectorAll("button[data-toggle]").forEach(b => b.dataset.toggle = String(i));
      box.querySelectorAll("button[data-clear]").forEach(b => b.dataset.clear = String(i));
      box.querySelectorAll("button[data-remove]").forEach(b => b.dataset.remove = String(i));
      box.querySelector(".slot-name").textContent = `账户 ${i + 1}`;
    });
    // 少了一个槽，把"添加账户"按钮补回来
    if (editSlotCount < 3 && !$("m-slots").querySelector("[data-add]")) {
      const btn = document.createElement("button");
      btn.className = "add-slot";
      btn.type = "button";
      btn.dataset.add = "1";
      btn.innerHTML = `<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3.5v9M3.5 8h9"/></svg>添加账户`;
      $("m-slots").appendChild(btn);
    }
    syncSlotExpansion();
  }
});

// 按 expandedSlot 同步每个槽的展开状态（不重画，只切 class 和 hidden）
function syncSlotExpansion() {
  $("m-slots").querySelectorAll(".slot-edit").forEach(box => {
    const i = Number(box.dataset.slot);
    const on = i === expandedSlot;
    box.classList.toggle("expanded", on);
    box.querySelector(".grid").hidden = !on;
  });
}

// 用户一旦手动改过密码框，异步回来的原密码就不再覆盖它
$("m-slots").addEventListener("input", e => {
  if (e.target.dataset.f === "pass")
    e.target.dataset.edited = "1";
});

$("modal-save").addEventListener("click", () => {
  const name = $("m-name").value.trim();
  if (!name) {
    $("m-name").focus();
    return;
  }

  const msg = {
    t: editingId ? "save" : "create",
    name,
    remark: $("m-remark").value.trim()
  };
  if (editingId) msg.id = editingId;

  // 搜索框里还有词时，新建/改名后的项目很可能被过滤掉——列表里看不见，
  // 用户会以为"没保存成功"。提交时一律清掉搜索词，保证这条一定可见。
  if (query) {
    query = "";
    $("q").value = "";
  }

  // 界面上可能只有 1~2 个槽，但密库格式固定 3 个：
  // 缺的那几个提交空槽，宿主侧不用改。
  for (let i = 0; i < 3; ++i) {
    const box = $("m-slots").querySelector(`.slot-edit[data-slot="${i}"]`);
    if (!box) {
      msg["s" + i] = { user: "", remark: "", pass: "" };
      continue;
    }
    const user = box.querySelector('[data-f="user"]').value.trim();
    const remark = box.querySelector('[data-f="remark"]').value.trim();
    const passInp = box.querySelector('[data-f="pass"]');
    const pass = passInp.value;

    // 未修改的密码由宿主从原始槽位提取，读取尚未返回也能安全保存。
    const source = Number(box.dataset.source);
    const passField = source >= 0 && passInp.dataset.edited !== "1" ? null : pass;
    msg["s" + i] = { user, remark, pass: passField, ...(source >= 0 ? {source} : {}) };

  }

  if (savingRequest) return;
  savingRequest = "save:" + editorSession;
  msg.request = savingRequest;
  $("modal-save").disabled = true;
  $("overlay").querySelector(".modal-body").inert = true;
  send(msg);
});

// 新增/保存后由宿主推新快照；这里只负责关窗
document.addEventListener("keydown", e => {
  if (e.key !== "Escape") return;
  if ($("overlay").hidden === false) { requestCloseEditor(); return; }
  if ($("imp").hidden === false) { closeImport(); return; }
  if ($("ipwd").hidden === false) {
    $("ipwd").hidden = true;
    $("ipwd-input").value = "";
    send({ t: "import_cancel" });
    return;
  }
  if ($("pwd").hidden === false) { $("pwd-cancel").click(); return; }
  if ($("recpwd").hidden === false) { $("recpwd-cancel").click(); return; }
  if ($("reccode").hidden === false) { $("reccode-cancel").click(); return; }
  if ($("recover").hidden === false) { $("recover-cancel").click(); return; }
  if ($("recnew").hidden === false) { $("recnew-cancel").click(); return; }
  if ($("confirm").hidden === false) $("confirm").hidden = true;
});

// ---------------------------------------------------------------- 闲置信号
// 宿主按"最后一次活动"计时，闲置 5 分钟自动抹掉密钥并回到登录页。
// 这里只负责告诉宿主"人还在动"。节流到 15 秒一次——鼠标移动事件太密，
// 不节流会让 IPC 通道一直忙，而宿主那边 20 秒才查一次，多报也没用。
let lastTouchAt = 0;
function reportActivity() {
  const now = Date.now();
  if (now - lastTouchAt < 15000) return;
  lastTouchAt = now;
  send({ t: "touch" });
}
["mousemove", "mousedown", "keydown", "wheel", "touchstart"].forEach(ev =>
  document.addEventListener(ev, reportActivity, { passive: true })
);

// ---------------------------------------------------------------- 跨机器导入
//
// 三步：预检（宿主做，不需要密码）→ 输对方密码 → 预览勾选。
// 前端全程不接触密码，只接触"这套账户有密码"这个布尔值。

function showImportPreview(msg) {
  impData = {
    fileName: msg.fileName || "",
    summary: msg.summary || {},
    entries: (msg.entries || []).map(e => ({
      id: e.id,
      name: e.name,
      action: e.action,
      sameName: e.sameName,
      localName: e.localName,
      copies: e.copies || 0,
      metadataConflict:!!e.metadataConflict, incomingRemark:e.incomingRemark||"",localRemark:e.localRemark||"",useIncomingMetadata:false,
      checked: e.action !== "skip",
      slots: (e.slots || []).map(s => ({
        user: s.user,
        remark: s.remark,
        hasPass: s.hasPass,
        srcSlot: s.srcSlot,
        duplicate: s.duplicate,
        checked: !s.excluded && !s.duplicate
      }))
    }))
  };
  $("imp-ok").disabled=false; $("imp-tree").inert=false;
  renderImport();
  $("imp").hidden = false;
}

function renderImport() {
  const d = impData;
  if (!d) return;

  $("imp-file").textContent = d.fileName;
  renderImportSummary();

  const tree = $("imp-tree");
  if (!d.entries.length) {
    tree.innerHTML = '<div class="imp-empty">没有可导入的内容</div>';
    return;
  }

  tree.innerHTML = d.entries.map((e, i) => {
    let badge;
    if (e.action === "merge" || e.sameName) {
      badge = `<span class="imp-badge">并入「${esc(e.localName || e.name)}」</span>`;
    } else if (e.copies > 0) {
      badge = `<span class="imp-badge warn">另存 ${e.copies} 条副本</span>`;
    } else {
      badge = `<span class="imp-badge">新项目</span>`;
    }

    const accs = e.slots.map((a, j) => {
      const tag = a.duplicate ? "已存在，不会重复添加"
        : (a.hasPass ? "有密码" : "无密码");
      return `<label class="imp-acc">
        <input type="checkbox" data-proj="${i}" data-acc="${j}"${a.checked ? " checked" : ""}${e.checked ? "" : " disabled"}>
        <span class="who">${esc(a.user)}</span>
        <span class="tag">${esc(tag)}</span>
        ${a.remark ? `<span class="tag">· ${esc(a.remark)}</span>` : ""}
      </label>`;
    }).join("");

    return `<div class="imp-proj${e.checked ? "" : " imp-skip"}">
      <label class="imp-proj-head">
        <input type="checkbox" data-proj="${i}"${e.checked ? " checked" : ""}>
        <span>${esc(e.name)}</span>
        ${badge}
      </label>
      ${e.metadataConflict ? `<label class="imp-acc"><input type="checkbox" data-proj="${i}" data-metadata${e.useIncomingMetadata?" checked":""}${e.checked?"":" disabled"}>使用外部名称和备注（默认保留本机）</label><p class="imp-acc">本机：${esc(e.localName)} / ${esc(e.localRemark)}<br>外部：${esc(e.name)} / ${esc(e.incomingRemark)}</p>` : ""}
      ${accs}
    </div>`;
  }).join("");
}

$("imp-tree").addEventListener("change", ev => {
  const t = ev.target;
  if (!impData || t.type !== "checkbox") return;
  const i = Number(t.dataset.proj);
  const proj = impData.entries[i];
  if (!proj) return;

  if(t.hasAttribute("data-metadata")){proj.useIncomingMetadata=t.checked;}
  else if (t.dataset.acc === undefined) {
    // 勾/取消整个项目：取消时它的账户也一并不导
    proj.checked = t.checked;
    renderImport();
  } else {
    proj.slots[Number(t.dataset.acc)].checked = t.checked;
  }
  $("imp-sum").textContent="正在重新计算本次选项…";
  send({t:"import_plan",...importOptions(),request:++importRevision});
});

function closeImport() {
  $("imp").hidden = true;
  $("imp-tree").replaceChildren();
  $("imp-file").textContent = "";
  $("imp-sum").textContent = "";
  impData = null;
  send({ t: "import_cancel" });
}

$("imp-cancel").addEventListener("click", closeImport);

function importOptions() {
 const mergeSameName=[],skipProjects=[],excludeSlots=[],useIncomingMetadata=[];
 for(const e of impData.entries){if(!e.checked){skipProjects.push(e.id);continue;}if(e.sameName)mergeSameName.push(e.id);if(e.useIncomingMetadata)useIncomingMetadata.push(e.id);
 for(const a of e.slots)if(!a.checked)excludeSlots.push({id:e.id,slot:a.srcSlot});}
 return {mergeSameName,skipProjects,excludeSlots,useIncomingMetadata};
}
function renderImportSummary(){
 if(!impData)return;const s=impData.summary;
 $("imp-sum").textContent=`账户取并集（不传播删除）；新增 ${s.added||0} 个项目、${s.slotsAdded||0} 套账户、${s.overflow||0} 个溢出副本，更新 ${s.metadataChanged||0} 个项目名称/备注`;
}
$("imp-ok").addEventListener("click",()=>{
 if(!impData||$("imp-ok").disabled)return;
 $("imp-ok").disabled=true;$("imp-tree").inert=true;
 send({t:"import_commit",...importOptions()});
});

// ---- 输对方机器的密码 ----

function submitImportPwd() {
  const pwd = $("ipwd-input").value;
  if (!pwd) return;
  $("ipwd-err").textContent = "";
  send({ t: "import_unlock", pwd });
}

$("ipwd-ok").addEventListener("click", submitImportPwd);
$("ipwd-input").addEventListener("keydown", e => {
  if (e.key === "Enter") submitImportPwd();
});
$("ipwd-cancel").addEventListener("click", () => {
  $("ipwd").hidden = true;
  $("ipwd-input").value = "";
  send({ t: "import_cancel" });
});

// ---------------------------------------------------------------- 修改主密码

function openChangePwd() {
  $("pwd-old").value = "";
  $("pwd-new").value = "";
  $("pwd-new2").value = "";
  $("pwd-err").textContent = "";
  $("pwd").hidden = false;
  setTimeout(() => $("pwd-old").focus(), 30);
}

$("pwd-ok").addEventListener("click", () => {
  const oldp = $("pwd-old").value;
  const n1 = $("pwd-new").value;
  const n2 = $("pwd-new2").value;

  if (!oldp) { $("pwd-err").textContent = "请输入当前密码"; return; }
  if (n1.length < 6) { $("pwd-err").textContent = "新密码至少 6 位"; return; }
  if (n1 !== n2) { $("pwd-err").textContent = "两次输入的新密码不一致"; return; }

  send({ t: "rekey", old: oldp, new: n1 });
});

$("pwd-cancel").addEventListener("click", () => {
  $("pwd").hidden = true;
  $("pwd-old").value = "";
  $("pwd-new").value = "";
  $("pwd-new2").value = "";
});

// ---------------------------------------------------------------- 恢复码
//
// 恢复码是主密码之外的第二把钥匙：它能解出同一个数据密钥。
// 所以它的保管要求和密码一样严——抄在纸上、和电脑分开。

let recCodeText = "";

function openSetRecovery() {
  $("recpwd-input").value = "";
  $("recpwd-err").textContent = "";
  $("recpwd").hidden = false;
  setTimeout(() => $("recpwd-input").focus(), 30);
}

function showRecoveryCode(code) {
  recCodeText = code || "";
  $("reccode-text").textContent = recCodeText;
  $("rec-confirm").value = "";
  $("reccode-err").textContent = "";
  $("recpwd").hidden = true;
  $("recpwd-input").value = "";
  $("reccode").hidden = false;
  setTimeout(() => $("rec-confirm").focus(), 30);
}

// 抄写核对时统一口径：去分隔符、转大写、容错 O/0 与 I/L
function normCode(s) {
  return String(s || "")
    .replace(/[-_\s]/g, "")
    .toUpperCase()
    .replace(/O/g, "0")
    .replace(/[IL]/g, "1");
}

function showRecError(text) {
  if (!$("recpwd").hidden) { $("recpwd-err").textContent = text; return; }
  if (!$("recover").hidden) { $("recover-err").textContent = text; return; }
  if (!$("recnew").hidden) { $("recnew-err").textContent = text; return; }
  toast("err", text);
}

$("btn-recover").addEventListener("click", () => {
  $("recover-input").value = "";
  $("recover-err").textContent = "";
  $("recover").hidden = false;
  setTimeout(() => $("recover-input").focus(), 30);
});

$("recpwd-ok").addEventListener("click", () => {
  const pwd = $("recpwd-input").value;
  if (!pwd) { $("recpwd-err").textContent = "请输入当前主密码"; return; }
  $("recpwd-err").textContent = "";
  send({ t: "set_recovery", pwd });
});
$("recpwd-input").addEventListener("keydown", e => {
  if (e.key === "Enter") $("recpwd-ok").click();
});
$("recpwd-cancel").addEventListener("click", () => {
  $("recpwd").hidden = true;
  $("recpwd-input").value = "";
});

$("reccode-ok").addEventListener("click", () => {
  const want = normCode(recCodeText).slice(-8);
  const got = normCode($("rec-confirm").value);
  if (got.length < 8) {
    $("reccode-err").textContent = "请输入最后 8 个字符";
    return;
  }
  if (got.slice(-8) !== want) {
    $("reccode-err").textContent = "对不上，请再核对一下抄写的最后 8 位";
    return;
  }
  $("reccode").hidden = true;
  recCodeText = "";
  $("reccode-text").textContent = "";
  $("rec-confirm").value = "";
  toast("ok", "恢复码已生效。请把那张纸收在安全的地方。");
});

$("reccode-cancel").addEventListener("click", () => {
  $("reccode").hidden = true;
  recCodeText = "";
  $("reccode-text").textContent = "";
  $("rec-confirm").value = "";
  // 码已经生成并写进密库了，没抄下来等于没有——必须说明白
  toast("warn", "恢复码已生成但没有确认抄写。忘了密码时它救不了你，建议重新设一次。");
});

$("recover-ok").addEventListener("click", () => {
  const code = $("recover-input").value.trim();
  if (!code) { $("recover-err").textContent = "请输入恢复码"; return; }
  $("recover-err").textContent = "";
  send({ t: "recover", code });
});
$("recover-input").addEventListener("keydown", e => {
  if (e.key === "Enter") $("recover-ok").click();
});
$("recover-cancel").addEventListener("click", () => {
  $("recover").hidden = true;
  $("recover-input").value = "";
});

$("recnew-ok").addEventListener("click", () => {
  const n1 = $("recnew-1").value;
  const n2 = $("recnew-2").value;
  if (n1.length < 6) { $("recnew-err").textContent = "新密码至少 6 位"; return; }
  if (n1 !== n2) { $("recnew-err").textContent = "两次输入的新密码不一致"; return; }
  $("recnew-err").textContent = "";
  send({ t: "recover_set_password", new: n1 });
});
$("recnew-cancel").addEventListener("click", () => {
  $("recnew").hidden = true;
  $("recnew-1").value = "";
  $("recnew-2").value = "";
  send({ t: "recover_cancel" });
});

// ---------------------------------------------------------------- 软键盘适配
//
// Android 15（API 35）强制 edge-to-edge，Manifest 里的 adjustResize
// 不再自动缩小 WebView——键盘弹起来直接盖住弹层底部，保存按钮点不到。
//
// visualViewport.height 是"真正还能看见的高度"，键盘弹起时它会变小。
// 把它写进 CSS 变量，弹层的 max-height 跟着变。
(function setupViewportHeight() {
  const vv = window.visualViewport;
  if (!vv) return;

  const apply = () => {
    document.documentElement.style.setProperty("--vvh", vv.height + "px");
  };

  vv.addEventListener("resize", apply);
  vv.addEventListener("scroll", apply);
  apply();
})();

// ---------------------------------------------------------------- 底部操作栏
//
// Android 上没有菜单栏（tauri::menu 在移动端不存在），桌面靠菜单承载的
// 功能必须有界面入口。底栏只出现在窄屏（CSS 控制），桌面仍是原生菜单。
//
// 注意没放「退出程序」：Android 的应用不该有退出按钮，系统会自己管进程，
// 用户按返回键退出即可。硬加一个只会让人觉得别扭。

$("tab-new").addEventListener("click", () => openEditor(null));
$("tab-import").addEventListener("click", () => send({ t: "import" }));
$("tab-lock").addEventListener("click", () => send({ t: "lock" }));
$("btn-more").addEventListener("click", () => { $("sheet").hidden=false; });
$("tab-more").addEventListener("click", () => {
  $("sheet").hidden = false;
});

const closeSheet = () => { $("sheet").hidden = true; };

$("sheet-cancel").addEventListener("click", closeSheet);

$("sheet-export").addEventListener("click", () => { closeSheet(); send({ t: "export" }); });
$("btn-export").addEventListener("click", () => send({ t: "export" }));

$("sheet-changePwd").addEventListener("click", () => {
  closeSheet();
  openChangePwd();
});

$("sheet-recovery").addEventListener("click", () => {
  closeSheet();
  openSetRecovery();
});

$("sheet-theme").addEventListener("click", () => {
  closeSheet();
  const now = document.documentElement.dataset.theme;
  applyTheme(now === "dark" ? "light" : "dark");
});

$("sheet-about").addEventListener("click", () => {
  closeSheet();
  send({ t: "about" });
});

// 点遮罩空白处关掉抽屉
$("sheet").addEventListener("click", e => {
  if (e.target === $("sheet")) closeSheet();
});

// ---------------------------------------------------------------- 安卓返回键 / 侧滑返回
//
// Tauri 的 back-button 事件统一处理实体键与返回手势。
// 弹层之间切换只修改 UI；不再用 history.back 关闭弹层，避免异步 popstate
// 再次取消新打开的导入预览或下层编辑器。

const overlayEls = [...document.querySelectorAll(".overlay")];

// DOM 末尾的可见弹层优先，支持编辑器上方的放弃确认。
const topLayer = () => [...overlayEls].reverse().find(el => !el.hidden) || null;

// 返回一层。返回 true 表示这次返回被我们消费掉了。
function handleBack() {
  const top = topLayer();
  if (top) {
    if (top.id === "overlay") requestCloseEditor();
    else if (top.id === "imp") closeImport();
    else {
      const cancel = $(top.id + "-cancel");
      if (cancel) cancel.click();
      else { top.hidden = true; top.querySelectorAll("input").forEach(el => el.value = ""); }
    }
    return true;
  }
  if (document.body.classList.contains("showing-detail")) {
    document.body.classList.remove("showing-detail");
    wantDetail = false;
    return true;
  }
  return false;
}

// 安卓实体/虚拟返回键。
//
// Android 返回键是移动插件通道，普通 event.listen 不会注册到 AppPlugin。
// 先逐层关闭界面，列表页再次返回才退出。
function registerBackButton(api = tauriApi) {
  if (!api?.app?.onBackButtonPress) return Promise.resolve();
  return api.app.onBackButtonPress(async () => {
    if (handleBack()) return;
    // 退无可退，才真的退出
    try {
      await api.app.exit(0);
    } catch (_) {
      // 没有该权限就什么都不做，总比把用户扔回桌面上强
    }
  });
}

// ---------------------------------------------------------------- 长按拖动排序
//
// 手机上把上移/下移按钮隐藏了，排序只能靠拖：
//   按住 1 秒 → 项目浮起 → 上下拖 → 松手落定
//
// 实现要点：
//   · 事件绑在 #list 上做委托，因为 renderList() 每次会重建所有 .item
//   · 长按计时器在 touchmove 里清掉——用户只是想滚列表时不能误触发拖拽
//   · 拖拽中才 preventDefault，否则正常滚动会被吃掉
//   · 松手发一条 reorder（带目标下标），宿主一次落盘，不用逐条发 up/down

(function setupDragSort() {
  const list = $("list");

  let pressTimer = null;
  let dragEl = null;        // 正在拖的那一项
  let items = [];           // 拖拽开始时的所有项（DOM 顺序）
  let fromIndex = 0;        // 起始下标
  let curIndex = 0;         // 当前落点下标
  let startY = 0;
  let rowTops = [], rowHeights = [];
  let minDrag = 0, maxDrag = 0, maxScroll = 0;
  let autoFrame=0, fingerY=0, scrollStart=0;
  let touchMoved = false;   // 手指是否移动过（超过 8px 阈值）            // 相邻两项的间距（含 gap），用来换算拖了几格

  const clearPress = () => {
    if (pressTimer) {
      clearTimeout(pressTimer);
      pressTimer = null;
    }
  };

  const resetDrag = () => {
    cancelAnimationFrame(autoFrame);
    if (dragEl) {
      dragEl.classList.remove("dragging");
      dragEl.style.transform = "";
    }
    items.forEach(el => { el.style.transform = ""; el.classList.remove("shifting"); });
    dragEl = null;
    items = [];
    fromIndex = curIndex = 0;
    list.classList.remove("sorting");
  };

  list.addEventListener("touchstart", ev => {
    if (ev.touches.length > 1) { clearPress(); touchMoved = true; resetDrag(); return; }
    const item = ev.target.closest(".item");
    if (!item) return;

    // 拖拽后的那次 click 要挡掉，但用"下次触摸开始时清除"而不是定时器：
    // 安卓的 click 延迟不稳定，定时器可能会在 click 之前就失效
    // （P10 上就出现了"拖完松手直接跳进详情页"）。
    delete document.body.dataset.justDragged;

    startY = ev.touches[0].clientY;
    touchMoved = false;

    if(query)return;
    pressTimer = setTimeout(() => {
      dragEl = item;
      list.classList.add("sorting");
      items = [...list.querySelectorAll(".item")];
      fromIndex = curIndex = items.indexOf(item);
      const rects = items.map(el => el.getBoundingClientRect());
      rowTops = rects.map(r => r.top);
      rowHeights = rects.map(r => r.height);
      minDrag = rowTops[0] - rowTops[fromIndex];
      maxDrag = rects.at(-1).bottom - rects[fromIndex].bottom;
      // 仅使用拖动前的真实内容范围，不能让 transform 产生的溢出扩大滚动上限。
      maxScroll = Math.max(0, list.scrollHeight - list.clientHeight);
      scrollStart=list.scrollTop;fingerY=startY;
      const autoScroll=()=>{
        if(!dragEl)return;
        const r=list.getBoundingClientRect(); const speed=fingerY<r.top+48?-9:fingerY>r.bottom-48?9:0;
        if(speed){list.scrollTop=Math.max(0,Math.min(maxScroll,list.scrollTop+speed));updateDrag(fingerY);}
        autoFrame=requestAnimationFrame(autoScroll);
      };
      autoFrame=requestAnimationFrame(autoScroll);
      dragEl.classList.add("dragging");

      // 标记"这次是拖拽"，让随后的 click 不作数（在 touchmove 里 reset 之前先置位）
      document.body.dataset.justDragged = "1";

      // 轻微震动，让用户知道"拿起来了"
      if (navigator.vibrate) navigator.vibrate(15);
    }, 1000);
  }, { passive: true });

  list.addEventListener("touchmove", ev => {
    if (ev.touches.length !== 1) { clearPress(); touchMoved = true; resetDrag(); return; }
    // 只要手指动了就取消长按计时（除非已经在拖）
    if (!dragEl) {
      // 还没进拖拽：记下手指有没有明显移动。
      // 移动过就是"在滚列表"，既不该触发长按，也不算点击。
      if (Math.abs(ev.touches[0].clientY - startY) > 8) touchMoved = true;
      clearPress();
      return;
    }

    // 拖动中要吃掉默认行为，否则页面会跟着滚
    ev.preventDefault();

    fingerY=ev.touches[0].clientY; updateDrag(fingerY);
  }, { passive: false });

  function updateDrag(y) {
    const dy = Math.max(minDrag, Math.min(maxDrag, y - startY + list.scrollTop - scrollStart));
    dragEl.style.transform = `translateY(${dy}px)`;

    // 按原始行的中点换位，兼容不同行高；两端始终停在实际列表内。
    let target = fromIndex;
    if (dy > 0) {
      while (target < items.length - 1 && rowTops[fromIndex] + rowHeights[fromIndex] + dy > rowTops[target + 1] + rowHeights[target + 1] / 2) target++;
    } else {
      while (target > 0 && rowTops[fromIndex] + dy < rowTops[target - 1] + rowHeights[target - 1] / 2) target--;
    }
    if (target === curIndex) return;

    // 让位必须只看「起点 fromIndex」和「当前落点 target」，不能看 curIndex。
    //
    // 之前用 curIndex 做增量判断，分多次往下拖时会出问题：
    //   第 1 步 target=1 → 第 2 项上移一格
    //   第 2 步 target=2 → 第 2 项不再满足条件，transform 被清空回到原位，
    //                      第 3 项此时才上移一格 → 两项重叠在同一个位置。
    // 也就是用户看到的「222 和 333 重合」。
    items.forEach((el, i) => {
      if (el === dragEl) return;
      let shift = 0;
      if (target > fromIndex) {
        // 往下拖：起点和落点之间的都往上让一格
        if (i > fromIndex && i <= target) shift = -rowHeights[fromIndex];
      } else if (target < fromIndex) {
        // 往上拖：落点和起点之间的都往下让一格
        if (i >= target && i < fromIndex) shift = rowHeights[fromIndex];
      }

      if (shift) {
        el.style.transform = `translateY(${shift}px)`;
        el.classList.add("shifting");
      } else {
        el.style.transform = "";
        el.classList.remove("shifting");
      }
    });

    curIndex = target;
  }

  const finish = (ev) => {
    clearPress();
    document.body.dataset.lastTouch = String(Date.now());

    // 没进入拖拽、手指也没移动 → 这一次是点击，自己处理。
    // 不再等浏览器在 touchend 后补发的 click：那个时机完全不可控，
    // K90 上用标志位、时间戳、pointer-events 三种办法都挡不住。
    if (!dragEl) {
      if (!touchMoved && ev && ev.target) {
        const item = ev.target.closest(".item");
        if (item) selectProject(item.dataset.id);
      }
      return;
    }

    // 取值必须在 resetDrag 之前——它会把 fromIndex / curIndex 一起清零，
    // 之前就是踩了这个坑：send 放在 reset 之后，to 永远是 0，等于原地不动。
    const id = dragEl.dataset.id;
    const toIndex = curIndex;
    const moved = toIndex !== fromIndex;

    // 先清掉视觉状态，再请求重排（renderList 会重建 DOM，不清会残留）
    resetDrag();

    if (moved) {
      send({ t: "reorder", id, to: toIndex });
    }

    // 拖拽结束后系统补发的那次 click 还是会漏过来（K90 上实测）。
    // 不再靠标志位猜时机，直接让列表在 400ms 内不接收指针事件——
    // click 压根不会派发，比"收到再丢弃"更彻底。
    list.style.pointerEvents = "none";
    setTimeout(() => { list.style.pointerEvents = ""; }, 400);

  };

  list.addEventListener("touchend", finish);
  list.addEventListener("touchcancel", () => { clearPress(); touchMoved=true; resetDrag(); document.body.dataset.lastTouch=String(Date.now()); });
  document.addEventListener("visibilitychange", () => { if (document.hidden) { clearPress(); touchMoved=true; resetDrag(); } });
})();

// Dialog focus and accessible labels are shared by every flow, including dynamic account rows.
function labelInputs(root=document){root.querySelectorAll('input:not([type=checkbox]),textarea').forEach(el=>{if(!el.hasAttribute('aria-label'))el.setAttribute('aria-label',el.closest('label')?.textContent.trim()||el.placeholder||el.id||'输入');});}
labelInputs();
const focusOrigins=new WeakMap();
const focusObserver=new MutationObserver(records=>{
 for(const {target:el} of records){if(!el.classList.contains('overlay'))continue;
 if(!el.hidden){el.inert=false;focusOrigins.set(el,document.activeElement);labelInputs(el);const focus=el.querySelector('input:not([disabled]),button:not([disabled]),[tabindex]');queueMicrotask(()=>focus?.focus());}
 else {el.inert=false;const origin=focusOrigins.get(el);if(origin?.isConnected&&!origin.closest('[hidden]'))origin.focus();}
 }
 const top=topLayer();
 $('app').inert=!!top;$('login').inert=!!top;$('desktop-chrome').inert=!!top;if(top)closeDesktopMenu();
 overlayEls.forEach(el=>el.inert=!!top && el!==top);
});
overlayEls.forEach(el=>focusObserver.observe(el,{attributes:true,attributeFilter:['hidden']}));
document.addEventListener('keydown',e=>{
 if(e.key!=='Tab')return;const top=topLayer();if(!top)return;
 const nodes=[...top.querySelectorAll('button,input,textarea,select,[tabindex="0"]')].filter(el=>!el.disabled&&el.getClientRects().length&&!el.closest('[inert]'));
 if(!nodes.length)return;const first=nodes[0],last=nodes.at(-1);
 if(e.shiftKey&&(document.activeElement===first||!top.contains(document.activeElement))){e.preventDefault();last.focus();}
 else if(!e.shiftKey&&(document.activeElement===last||!top.contains(document.activeElement))){e.preventDefault();first.focus();}
});
$('list').addEventListener('keydown',e=>{
 const item=e.target.closest('.item');if(!item)return;
 if(['Enter',' '].includes(e.key)){e.preventDefault();selectProject(item.dataset.id);}
 const items=[...$('list').querySelectorAll('.item')];let idx=items.indexOf(item);
 if(['ArrowDown','ArrowUp','Home','End'].includes(e.key)){e.preventDefault();idx=e.key==='Home'?0:e.key==='End'?items.length-1:Math.max(0,Math.min(items.length-1,idx+(e.key==='ArrowDown'?1:-1)));items[idx]?.focus();}
});
document.addEventListener('visibilitychange',()=>{if(document.hidden){secrets={};detailReveals.clear();setLoginPasswordVisible(false);document.querySelectorAll('[data-f="pass"]').forEach(el=>setEditorPasswordVisible(el,false));if(!$('app').hidden)renderDetail();}});
const headResize=new ResizeObserver(entries=>{for(const {target} of entries)target.closest('.modal').style.setProperty('--head-h',target.offsetHeight+'px');});
headResize.observe($('overlay').querySelector('.modal-head'));

// Windows 主题菜单：文字入口、中文读屏名称与完整键盘导航。
const desktopMenus = [
 {id:'file',label:'文件',icon:'file',key:'f',items:[
  ['new','新增项目','plus','Ctrl+N'],['delete','删除项目','trash','Del'],null,
  ['import','导入外部密库…','import'],['export','导出加密备份…','export'],null,
  ['changePwd','修改主密码…','key'],['setRecovery','设置恢复码…','shield'],null,
  ['lock','锁定密库','lock','Ctrl+L'],['quit','退出','enter','Ctrl+Q']
 ]},
 {id:'view',label:'外观',icon:'palette',key:'v',items:[['theme-dark','深色模式','moon'],['theme-light','浅色模式','sun']]},
 {id:'help',label:'帮助',icon:'help',key:'h',items:[['about','关于 SecBack','help']]}
];
let desktopMenuOpen = null;
function syncDesktopMenu() {
 const locked=$('app').hidden;
 document.querySelectorAll('[data-menu-action]').forEach(button=>{
  const action=button.dataset.menuAction;
  button.disabled=locked&&!['theme-dark','theme-light','about','quit'].includes(action)||action==='delete'&&!S.selected;
  if(action.startsWith('theme-'))button.setAttribute('aria-checked',String(action==='theme-'+document.documentElement.dataset.theme));
 });
}
function closeDesktopMenu(restore=false) {
 const origin=desktopMenuOpen&&$('menu-'+desktopMenuOpen);
 document.querySelectorAll('.desktop-dropdown').forEach(el=>el.hidden=true);
 document.querySelectorAll('.desktop-menu-trigger').forEach(el=>el.setAttribute('aria-expanded','false'));
 desktopMenuOpen=null;
 if(restore)origin?.focus();
}
function openDesktopMenu(id,focusItem=false,last=false) {
 syncDesktopMenu();closeDesktopMenu();desktopMenuOpen=id;
 const trigger=$('menu-'+id),panel=$('menu-panel-'+id);
 trigger.setAttribute('aria-expanded','true');panel.hidden=false;
 if(focusItem){const nodes=[...panel.querySelectorAll('button:not(:disabled)')];(last?nodes.at(-1):nodes[0])?.focus();}
}
function runDesktopAction(action) {
 closeDesktopMenu(true);
 if(action==='theme-dark'||action==='theme-light'){applyTheme(action.slice(6));return;}
 if(['about','quit','lock'].includes(action)){send({t:action});return;}
 if($('app').hidden||topLayer())return;
 if(action==='new')openEditor(null);
 if(action==='delete')askDelete();
 if(action==='import'||action==='export')send({t:action});
 if(action==='changePwd')openChangePwd();
 if(action==='setRecovery')openSetRecovery();
}
$('desktop-menubar').innerHTML=desktopMenus.map(menu=>
 '<div class="desktop-menu"><button type="button" role="menuitem" class="desktop-menu-trigger" id="menu-'+menu.id+'" aria-haspopup="menu" aria-controls="menu-panel-'+menu.id+'" aria-expanded="false" aria-keyshortcuts="Alt+'+menu.key+'">'+icon(menu.icon)+'<span>'+menu.label+'</span><svg class="menu-chevron" viewBox="0 0 16 16" aria-hidden="true"><path d="m5 6 3 3 3-3"/></svg></button><div class="desktop-dropdown" role="menu" aria-labelledby="menu-'+menu.id+'" id="menu-panel-'+menu.id+'" hidden>'+menu.items.map(item=>{
  if(!item)return '<div class="menu-separator" role="separator"></div>';
  const [action,label,symbol,shortcut]=item,theme=action.startsWith('theme-');
  return '<button type="button" tabindex="-1" role="'+(theme?'menuitemradio':'menuitem')+'" '+(theme?'aria-checked="false" ':'')+'data-menu-action="'+action+'">'+icon(symbol)+'<span>'+label+'</span>'+(shortcut?'<kbd>'+shortcut+'</kbd>':'')+(theme?'<span class="menu-check" aria-hidden="true">✓</span>':'')+'</button>';
 }).join('')+'</div></div>').join('');
$('desktop-menubar').addEventListener('click',e=>{
 const trigger=e.target.closest('.desktop-menu-trigger');
 if(trigger){const id=trigger.id.slice(5);if(desktopMenuOpen===id)closeDesktopMenu();else openDesktopMenu(id);return;}
 const item=e.target.closest('[data-menu-action]');if(item&&!item.disabled)runDesktopAction(item.dataset.menuAction);
});
$('desktop-menubar').addEventListener('pointerover',e=>{
 const trigger=e.target.closest('.desktop-menu-trigger');
 if(desktopMenuOpen&&trigger&&trigger.id!=='menu-'+desktopMenuOpen)openDesktopMenu(trigger.id.slice(5));
});
document.addEventListener('pointerdown',e=>{if(!e.target.closest('#desktop-menubar'))closeDesktopMenu();});
document.addEventListener('focusin',e=>{if(desktopMenuOpen&&!e.target.closest('#desktop-menubar'))closeDesktopMenu();});
window.addEventListener('blur',()=>closeDesktopMenu());
document.addEventListener('keydown',e=>{
 if(document.documentElement.dataset.platform!=='windows'||innerWidth<=700)return;
 if(topLayer())return;
 const key=e.key.toLowerCase();
 if(e.ctrlKey&&!e.altKey&&['l','q'].includes(key)){
  if(key==='l'&&$('app').hidden)return;e.preventDefault();runDesktopAction(key==='q'?'quit':'lock');return;
 }
 if((e.key==='F10'&&!e.shiftKey)||(e.altKey&&!e.ctrlKey&&desktopMenus.some(m=>m.key===key))){
  e.preventDefault();const menu=desktopMenus.find(m=>m.key===key)||desktopMenus[0];openDesktopMenu(menu.id,true);return;
 }
 const trigger=e.target.closest('.desktop-menu-trigger'),panel=e.target.closest('.desktop-dropdown');
 if(!trigger&&!panel)return;
 if(e.key==='Escape'){e.preventDefault();closeDesktopMenu(true);return;}
 if(e.key==='Tab'){closeDesktopMenu();return;}
 const menuId=trigger?trigger.id.slice(5):desktopMenuOpen;
 if(['ArrowLeft','ArrowRight'].includes(e.key)){
  e.preventDefault();const idx=desktopMenus.findIndex(m=>m.id===menuId),next=desktopMenus[(idx+(e.key==='ArrowRight'?1:desktopMenus.length-1))%desktopMenus.length];
  if(desktopMenuOpen)openDesktopMenu(next.id,true);else $('menu-'+next.id).focus();return;
 }
 if(['ArrowDown','ArrowUp','Home','End'].includes(e.key)){
  e.preventDefault();if(trigger){openDesktopMenu(menuId,true,e.key==='ArrowUp'||e.key==='End');return;}
  const nodes=[...panel.querySelectorAll('button:not(:disabled)')],idx=nodes.indexOf(document.activeElement);
  const next=e.key==='Home'?0:e.key==='End'?nodes.length-1:(idx+(e.key==='ArrowDown'?1:nodes.length-1))%nodes.length;nodes[next]?.focus();
 }
},true);
syncDesktopMenu();
