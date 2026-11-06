// SecBack 前端。
//
// 所有数据都由 C++ 宿主推送（snapshot / secret / toast），这里只负责渲染和把
// 用户操作翻译成命令发回去。前端【不持有】密码：快照里只有 hasPass 布尔值，
// 用户点"显示"或"复制"时才单独取一次，用完不缓存太久。

// 传输层：同一套 {t:...} 协议，两种承载。
//   Tauri    : invoke("host_message") 发、listen("host") 收
//   WebView2 : postMessage / message 事件
// 分开写是为了同一份前端既能跑桌面 Tauri，也能直接跑在手机的 Tauri 里。
const tauriApi = window.__TAURI__;
const webview2 = window.chrome && window.chrome.webview;

function send(obj) {
  const s = JSON.stringify(obj);
  if (tauriApi) tauriApi.core.invoke("host_message", { json: s });
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
let secrets = {};          // "id:slot" -> 明文密码（详情页"显示"用，临时）
let editingId = null;      // null 表示新增
let impData = null;        // 导入预览的临时状态

// 编辑弹层专用的 reveal 请求指纹。它和详情页的"显示"走同一条消息，
// 但拿到后只填输入框、不写进 secrets 缓存，少留一份明文在内存里。
const editReveals = new Set();

const $ = id => document.getElementById(id);

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
  $("login-pwd").value = "";
  $("login-go").disabled = false;
  setHint(hint || "", kind);
  setTimeout(() => $("login-pwd").focus(), 30);
}

function showMain() {
  $("login").hidden = true;
  $("app").hidden = false;
}

function doLogin() {
  const pwd = $("login-pwd").value;
  if (!pwd) {
    $("login-pwd").focus();
    return;
  }
  $("login-go").disabled = true;
  setHint("正在解锁…");
  send({ t: "login", pwd });
}

$("login-go").addEventListener("click", doLogin);
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
    $("count").textContent = "";
    return;
  }
  $("list-empty").hidden = true;

  list.innerHTML = items.map(p => {
    const cls = ["item"];
    if (p.id === S.selected) cls.push("active");
    if (p.imported) cls.push("imported");

    const sub = p.remark
      ? p.remark
      : (slotCount(p) ? slotCount(p) + " 套账户" : "暂无账户");

    return `<div class="${cls.join(" ")}" data-id="${p.id}">
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
  if (!p) return;

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
        <button class="mini" data-act="copy-user" data-slot="${i}">复制</button>
      </div>

      <div class="row">
        <span class="row-label">密码</span>
        <span class="row-value" id="pass-${i}">${esc(passText)}</span>
        <button class="mini" data-act="toggle-pass" data-slot="${i}">${revealed ? "隐藏" : "显示"}</button>
        <button class="mini" data-act="copy-pass" data-slot="${i}">复制</button>
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

  // 选区变了就丢弃旧的明文缓存，避免显示上一个项目的密码
  const keep = {};
  for (const k of Object.keys(secrets)) {
    if (k.startsWith(S.selected + ":")) keep[k] = secrets[k];
  }
  secrets = keep;

  render();
}

function onSecret(msg) {
  // 编辑弹层的请求：只填输入框，不写进 secrets 缓存
  const editKey = "edit:" + msg.id + ":" + msg.slot;
  if (editReveals.has(editKey)) {
    editReveals.delete(editKey);
    if (editingId === msg.id && $("overlay").hidden === false) {
      const box = $("m-slots").querySelector(`.slot-edit[data-slot="${msg.slot}"]`);
      if (box) {
        const inp = box.querySelector('[data-f="pass"]');
        // 用户已经动手输入过就别覆盖了
        if (inp && inp.dataset.edited !== "1") {
          inp.value = msg.pass;
          inp.placeholder = "";
          box.dataset.originalPass = msg.pass;
        }
      }
    }
    return;
  }

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
        ? "没有找到密库，将新建一份空的。\n位置：" + m.vaultPath
        : "";
      showLogin(hint);
      return;
    }

    if (m.t === "loggedIn") {
      showMain();
      return;
    }

    if (m.t === "loginError") {
      if (m.retryAfter > 0) {
        let left = Math.ceil(m.retryAfter / 1000);
        $("login-go").disabled = true;
        setHint("密码连续错误，请等待 " + left + " 秒", "warn");
        const tick = setInterval(() => {
          left -= 1;
          if (left <= 0) {
            clearInterval(tick);
            $("login-go").disabled = false;
            setHint("请重新输入主密码", "warn");
            $("login-pwd").focus();
          } else {
            setHint("密码连续错误，请等待 " + left + " 秒", "warn");
          }
        }, 1000);
      } else {
        $("login-go").disabled = false;
        setHint(m.text || "密码不正确", "error");
        $("login-pwd").value = "";
        $("login-pwd").focus();
      }
      return;
    }

    if (m.t === "loggedOut") {
      S = { projects: [], selected: "", canUp: false, canDown: false, defaultPwd: false };
      secrets = {};
      editReveals.clear();

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
      $("recnew").hidden = true;
      $("recpwd-input").value = "";
      $("recover-input").value = "";
      $("rec-confirm").value = "";
      $("recnew-1").value = "";
      $("recnew-2").value = "";
      recCodeText = "";
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

$("list").addEventListener("click", e => {
  const item = e.target.closest(".item");
  if (!item) return;
  const id = item.dataset.id;
  if (id === S.selected) return;
  S.selected = id;
  secrets = {};
  send({ t: "select", id });
  render();
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
      renderDetail();
    } else {
      // 先占位，收到 secret 再重绘
      secrets[key] = "…";
      renderDetail();
      send({ t: "reveal", id: S.selected, slot });
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
  const p = projectById(S.selected);
  if (!p) return;
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
  send({ t: "delete" });
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
  if (e.key === "Delete" && S.selected && document.activeElement.tagName !== "INPUT") {
    e.preventDefault();
    send({ t: "delete" });
  }
});

// ------------------------------------------------------------------ 编辑弹层

function slotEditHtml(i, s) {
  const empty = !s || s.empty;
  const hasPass = !empty && s.hasPass;

  // 密码直接显示原文：打开弹层时单独取一次（见 openEditor）。
  // 框里先空着并提示"读取中…"，拿到之前用户要是已经动手输入，就不再覆盖。
  return `<div class="slot-edit" data-slot="${i}">
    <div class="slot-edit-head">
      <span>账户 ${i + 1}${hasPass ? ' <em class="has-pass">已设置密码</em>' : ""}</span>
      <button class="clear" data-clear="${i}">清空</button>
    </div>
    <div class="grid">
      <input data-f="user"   placeholder="账号"          value="${esc(empty ? "" : s.user)}">
      <input data-f="pass"   type="text" autocomplete="off" spellcheck="false"
             placeholder="${hasPass ? "读取中…" : "密码"}"
             value="" data-haspass="${hasPass ? "1" : "0"}">
      <input data-f="remark" class="full" placeholder="备注" value="${esc(empty ? "" : s.remark)}">
    </div>
  </div>`;
}

function openEditor(p) {
  editingId = p ? p.id : null;
  $("modal-title").textContent = p ? "编辑项目" : "新增项目";
  $("m-name").value = p ? p.name : "";
  $("m-remark").value = p ? p.remark : "";
  $("m-slots").innerHTML = [0, 1, 2]
    .map(i => slotEditHtml(i, p ? p.slots[i] : null))
    .join("");

  // 已有密码的槽：按需取一次明文，填进输入框。
  // 走和详情页"显示"同一条消息，但记在 editReveals 里，收到后只填框、不进缓存。
  if (p) {
    for (let i = 0; i < 3; ++i) {
      if (!p.slots[i].empty && p.slots[i].hasPass) {
        editReveals.add("edit:" + p.id + ":" + i);
        send({ t: "reveal", id: p.id, slot: i });
      }
    }
  }

  $("overlay").hidden = false;
  setTimeout(() => $("m-name").focus(), 30);
}

function closeEditor() {
  $("overlay").hidden = true;
  // 框里现在是明文密码，关掉就清干净，别留在 DOM 里
  $("m-slots").innerHTML = "";
  editingId = null;
}

$("modal-x").addEventListener("click", closeEditor);
$("modal-cancel").addEventListener("click", closeEditor);

$("overlay").addEventListener("click", e => {
  if (e.target === $("overlay")) closeEditor();
});

$("m-slots").addEventListener("click", e => {
  const btn = e.target.closest("button[data-clear]");
  if (!btn) return;
  const box = btn.closest(".slot-edit");
  box.querySelectorAll("input").forEach(inp => {
    inp.value = "";
    inp.dataset.mask = "0";
  });
});

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

  for (let i = 0; i < 3; ++i) {
    const box = $("m-slots").querySelector(`.slot-edit[data-slot="${i}"]`);
    const user = box.querySelector('[data-f="user"]').value.trim();
    const remark = box.querySelector('[data-f="remark"]').value.trim();
    const passInp = box.querySelector('[data-f="pass"]');
    const pass = passInp.value;

    // 密码框里是明文原文，所以提交时靠"和原值比对"判断有没有改：
    //   没取到原值（原本就没密码）-> 有内容就是新密码，否则空
    //   和原值相同              -> null（不重写，避免无谓的一次密码落盘）
    //   不同（含被清空）        -> 提交当前值
    const original = box.dataset.originalPass;
    let passField;
    if (original === undefined)
      passField = pass.length > 0 ? pass : "";
    else if (pass === original)
      passField = null;
    else
      passField = pass;

    msg["s" + i] = { user, remark, pass: passField };
  }

  send(msg);
  closeEditor();
});

// 新增/保存后由宿主推新快照；这里只负责关窗
document.addEventListener("keydown", e => {
  if (e.key !== "Escape") return;
  if ($("overlay").hidden === false) { closeEditor(); return; }
  if ($("imp").hidden === false) { closeImport(); return; }
  if ($("ipwd").hidden === false) {
    $("ipwd").hidden = true;
    $("ipwd-input").value = "";
    send({ t: "import_cancel" });
    return;
  }
  if ($("pwd").hidden === false) { $("pwd").hidden = true; return; }
  if ($("recpwd").hidden === false) { $("recpwd").hidden = true; return; }
  if ($("reccode").hidden === false) { $("reccode-cancel").click(); return; }
  if ($("recover").hidden === false) { $("recover").hidden = true; return; }
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
  renderImport();
  $("imp").hidden = false;
}

function renderImport() {
  const d = impData;
  if (!d) return;

  $("imp-file").textContent = d.fileName;
  const s = d.summary;
  let sum = `新增 ${s.added || 0} 个项目 · 新增 ${s.slotsAdded || 0} 套账户`;
  if (s.overflow) sum += ` · ${s.overflow} 条账户装不下，会另存为副本`;
  $("imp-sum").textContent = sum;

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

  if (t.dataset.acc === undefined) {
    // 勾/取消整个项目：取消时它的账户也一并不导
    proj.checked = t.checked;
    renderImport();
  } else {
    proj.slots[Number(t.dataset.acc)].checked = t.checked;
  }
});

function closeImport() {
  $("imp").hidden = true;
  impData = null;
  send({ t: "import_cancel" });
}

$("imp-cancel").addEventListener("click", closeImport);
$("imp-x").addEventListener("click", closeImport);

$("imp-ok").addEventListener("click", () => {
  if (!impData) return;
  const mergeSameName = [];
  const skipProjects = [];
  const excludeSlots = [];

  for (const e of impData.entries) {
    if (!e.checked) {
      skipProjects.push(e.id);
      continue;
    }
    if (e.sameName) mergeSameName.push(e.id);
    for (const s of e.slots) {
      if (!s.checked) excludeSlots.push({ id: e.id, slot: s.srcSlot });
    }
  }

  send({ t: "import_commit", mergeSameName, skipProjects, excludeSlots });
  $("imp").hidden = true;
  impData = null;
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
  $("pwd").hidden = true;
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
  toast("ok", "恢复码已生效。请把那张纸收在安全的地方。");
});

$("reccode-cancel").addEventListener("click", () => {
  $("reccode").hidden = true;
  recCodeText = "";
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
