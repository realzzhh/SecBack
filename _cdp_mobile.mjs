// 移动端布局的快速验证工具。
//
// 用法: node _cdp_mobile.mjs <宽> <高> [css文件] [输出png]
//   例: node _cdp_mobile.mjs 393 851 out.png
//       node _cdp_mobile.mjs 393 851 patch.css mobile.png
//
// 为什么要有这个：移动布局全靠 @media (max-width: 700px)，跟平台无关、只跟视口宽度有关。
// 把视口设成手机尺寸，桌面版看到的就是安卓上的样子。
// 传了 css 文件就把它的内容注入成 <style> 追加到末尾——这样改样式不用重编 Rust，
// 十秒一轮，比「编译→打包→安装→截图」快三十倍。
// 验证通过后再把 CSS 写回 src/style.css。
//
// 需要应用以 WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 启动。

const width = parseInt(process.argv[2] || "393", 10);
const height = parseInt(process.argv[3] || "851", 10);
const cssFile = process.argv[4];
const out = process.argv[5] || "mobile.png";
// 第 6 个参数：在模拟视口下执行的 JS，结果会打印成 EVAL:
// CDP 的 Emulation 是会话级的——换个连接视口就还原了，
// 所以测量和截图必须在同一个连接里做，否则量到的是桌面尺寸。
const expr = process.argv[6];

import { readFileSync, writeFileSync } from "node:fs";

const port = process.env.CDP_PORT || 9222;
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find(t => t.type === "page");
if (!page) {
  console.error("没有找到页面，应用启动了没？");
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
    // 手机视口：deviceScaleFactor 设 2 让截图看清细节，mobile 让 CSS 按移动端处理
    await cmd("Emulation.setDeviceMetricsOverride", {
      width, height, deviceScaleFactor: 2, mobile: true,
      screenWidth: width, screenHeight: height,
    });

    if (cssFile) {
      const css = readFileSync(cssFile, "utf8");
      const js = `(() => {
        const old = document.getElementById("__patch");
        if (old) old.remove();
        const s = document.createElement("style");
        s.id = "__patch";
        s.textContent = ${JSON.stringify(css)};
        document.head.appendChild(s);
        return "注入 " + s.textContent.length + " 字符";
      })()`;
      const r = await cmd("Runtime.evaluate", { expression: js, returnByValue: true });
      console.log(r?.result?.value || "");
    }

    // 停一下让样式重排和动画稳定
    await new Promise(r => setTimeout(r, 350));

    if (expr) {
      const r = await cmd("Runtime.evaluate", {
        expression: expr, returnByValue: true, awaitPromise: true,
      });
      const v = r?.result?.value;
      // 走 stderr：stdout 要留给 base64，混在一起管道那头就没法解码了
      console.error("EVAL:", typeof v === "object" ? JSON.stringify(v) : v);
      await new Promise(r2 => setTimeout(r2, 250));
    }

    const shot = await cmd("Page.captureScreenshot", { format: "png" });
    const b64 = shot.data;

    // 输出 "-" 时把 base64 打到 stdout，交给 shell 重定向写文件。
    // 直接 writeFileSync 在这套沙箱里会被拦，绕一下更省事。
    if (out === "-") {
      // 等 flush 完成再退出，否则管道那头拿到的是 0 字节
      await new Promise(res => process.stdout.write(b64, () => res()));
    } else {
      writeFileSync(out, Buffer.from(b64, "base64"));
      console.log("截图 ->", out, `${width}x${height}`);
    }
  } catch (e) {
    console.error("出错:", e.message);
  } finally {
    ws.close();
  }
});
