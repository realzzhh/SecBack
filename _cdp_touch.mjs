// 用 CDP 的 Input.dispatchTouchEvent 模拟真实触摸序列。
//
// 为什么不用 adb shell input：adb 注入的 motionevent / draganddrop 会被
// WebView 当成普通点击（测拖拽排序时直接跳进了详情页），而 CDP 这条路径
// 和真手指走的是同一套事件，能真正触发 touchstart/touchmove/touchend。
//
// 用法: node _cdp_touch.mjs <x1> <y1> <x2> <y2> [按住毫秒=1200] [分几步=6]
// 例:   node _cdp_touch.mjs 200 180 200 320 1200 6

const x1 = Number(process.argv[2]);
const y1 = Number(process.argv[3]);
const x2 = Number(process.argv[4]);
const y2 = Number(process.argv[5]);
const holdMs = Number(process.argv[6] || 1200);
const steps = Number(process.argv[7] || 6);

const port = process.env.CDP_PORT || 9222;
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find(t => t.type === "page");
if (!page) { console.error("没有找到页面"); process.exit(1); }

const ws = new WebSocket(page.webSocketDebuggerUrl);
let seq = 0;
const pending = new Map();
const cmd = (method, params = {}) =>
  new Promise(res => {
    const id = ++seq;
    pending.set(id, res);
    ws.send(JSON.stringify({ id, method, params }));
  });

ws.addEventListener("message", ev => {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m.result); pending.delete(m.id); }
});

const sleep = ms => new Promise(r => setTimeout(r, ms));
const touch = (type, x, y) =>
  cmd("Input.dispatchTouchEvent", {
    type,
    touchPoints: type === "touchEnd" ? [] : [{ x, y, radiusX: 12, radiusY: 12, force: 1 }],
  });

ws.addEventListener("open", async () => {
  try {
    // 手机视口（和 _cdp_mobile.mjs 保持一致，坐标才对得上）
    await cmd("Emulation.setDeviceMetricsOverride", {
      width: 393, height: 851, deviceScaleFactor: 2, mobile: true,
      screenWidth: 393, screenHeight: 851,
    });
    await sleep(300);

    await touch("touchStart", x1, y1);
    await sleep(holdMs);                 // 按住，等长按计时器触发

    for (let i = 1; i <= steps; i++) {
      const t = i / steps;
      await touch("touchMove", x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
      await sleep(70);
    }

    await touch("touchEnd", x2, y2);
    await sleep(700);

    const order = await cmd("Runtime.evaluate", {
      expression: `[...document.querySelectorAll(".item")].map(el => el.querySelector(".item-name, .name, .t")?.textContent?.trim() || el.textContent.trim().slice(0,6)).join(" | ")`,
      returnByValue: true,
    });
    console.error("顺序:", order?.result?.value);
  } catch (e) {
    console.error("出错:", e.message);
  } finally {
    ws.close();
  }
});
