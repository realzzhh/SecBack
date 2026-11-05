// 通过 CDP 驱动 Tauri 的 WebView2：取值 / 截图。
// 用法: node _cdp.mjs <out.png|-> [js表达式]
//   输出路径写 "-" 就只取值不截图（别写 /dev/null：Windows 上会真建一个叫 nul 的文件）
//
// 需要应用以 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 启动。

const out = process.argv[2] || "shot.png";
const expr = process.argv[3];
const wantShot = out !== "-" && out !== "";

const targets = await (await fetch(`http://127.0.0.1:${process.env.CDP_PORT || 9222}/json`)).json();
const page = targets.find(t => t.type === "page");
if (!page) {
  console.error("没有找到页面");
  process.exit(1);
}

const ws = new WebSocket(page.webSocketDebuggerUrl);
let seq = 0;
const pending = new Map();

function cmd(method, params = {}) {
  return new Promise(res => {
    const id = ++seq;
    pending.set(id, res);
    ws.send(JSON.stringify({ id, method, params }));
  });
}

ws.addEventListener("message", ev => {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)(m.result);
    pending.delete(m.id);
  }
});

ws.addEventListener("open", async () => {
  try {
    if (expr) {
      const r = await cmd("Runtime.evaluate", {
        expression: expr,
        returnByValue: true,
        awaitPromise: true,
      });
      const v = r?.result?.value;
      console.log("EVAL:", typeof v === "string" ? v : JSON.stringify(v));
    }
    if (wantShot) {
      const s = await cmd("Page.captureScreenshot", { format: "png" });
      if (s?.data) {
        const fs = await import("node:fs");
        fs.writeFileSync(out, Buffer.from(s.data, "base64"));
        console.log("saved:", out);
      } else {
        console.error("截图失败", JSON.stringify(s));
      }
    }
  } catch (e) {
    console.error("ERR", e);
  }
  ws.close();
});
