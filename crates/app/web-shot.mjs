// Screenshots of the browser build in headless Chrome, without dependencies (Node 22+).
//
//   node crates/app/web-shot.mjs URL OUT.png ["STEPS"]
//
// STEPS, separated by `;`: `click X,Y` (CSS pixels), `rclick X,Y` (right button),
// `dclick X,Y` (double click), `drag X,Y X2,Y2`, `wheel X,Y DY`
// (a mouse wheel; DY in pixels, positive scrolls down), `key KEY` (a
// DOM key name: `r`, `Delete`, `Escape`, with modifiers `Meta+s`, `Control+Shift+z`),
// `type TEXT`, `reload` (the page; its storage stays), `downloads DIR` (where downloads
// go), `upload FILE` (answers the next file chooser), `wait MS`, `shot FILE.png`, `eval JS` (prints
// the result). The page's console is printed. The window is 1600×1000
// at 2× (like the desktop screenshots). CHROME overrides the browser's path.

import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [url, out, steps = ""] = process.argv.slice(2);
if (!url || !out) {
  console.error('usage: node web-shot.mjs URL OUT.png ["click X,Y; wait MS; shot F.png; eval JS"]');
  process.exit(2);
}
const chromePath =
  process.env.CHROME ??
  (process.platform === "darwin"
    ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    : "google-chrome");
const port = 9300 + Math.floor(Math.random() * 500);
const chrome = spawn(
  chromePath,
  [
    "--headless=new",
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${mkdtempSync(join(tmpdir(), "web-shot-"))}`,
    "--enable-unsafe-webgpu",
    "--window-size=1600,1000",
    "--force-device-scale-factor=2",
    "about:blank",
  ],
  { stdio: "ignore" },
);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function target() {
  for (let i = 0; i < 100; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {}
    await sleep(100);
  }
  throw new Error("Chrome didn't start");
}

const ws = new WebSocket(await target());
await new Promise((r) => ws.addEventListener("open", r));
let next = 0;
const uploads = [];
const waiting = new Map();
ws.addEventListener("message", (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id && waiting.has(msg.id)) {
    waiting.get(msg.id)(msg);
    waiting.delete(msg.id);
  } else if (msg.method === "Runtime.consoleAPICalled") {
    console.log(`console.${msg.params.type}:`, msg.params.args.map((a) => a.value ?? a.description).join(" "));
  } else if (msg.method === "Page.fileChooserOpened") {
    const files = uploads.splice(0, uploads.length);
    send("DOM.setFileInputFiles", { files, backendNodeId: msg.params.backendNodeId }).catch((e) => console.log(e.message));
  } else if (msg.method === "Runtime.exceptionThrown") {
    console.log("exception:", msg.params.exceptionDetails.exception?.description ?? msg.params.exceptionDetails.text);
  }
});
const send = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const id = ++next;
    waiting.set(id, (m) => (m.error ? reject(new Error(`${method}: ${m.error.message}`)) : resolve(m.result)));
    ws.send(JSON.stringify({ id, method, params }));
  });

async function shot(file) {
  const { data } = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(file, Buffer.from(data, "base64"));
  console.log(file);
}

try {
  await send("Runtime.enable");
  await send("Page.enable");
  await send("Emulation.setDeviceMetricsOverride", { width: 1600, height: 1000, deviceScaleFactor: 2, mobile: false });
  await send("Page.navigate", { url });
  await sleep(4000);
  for (const step of steps.split(";").map((s) => s.trim()).filter(Boolean)) {
    const [cmd, ...rest] = step.split(" ");
    const arg = rest.join(" ");
    if (cmd === "click" || cmd === "rclick") {
      const [x, y] = arg.split(",").map(Number);
      const button = cmd === "click" ? "left" : "right";
      for (const type of ["mouseMoved", "mousePressed", "mouseReleased"]) {
        await send("Input.dispatchMouseEvent", { type, x, y, button, clickCount: 1 });
        await sleep(50);
      }
      await sleep(300);
    } else if (cmd === "dclick") {
      const [x, y] = arg.split(",").map(Number);
      await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
      for (const clickCount of [1, 2]) {
        await send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount });
        await send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount });
      }
      await sleep(300);
    } else if (cmd === "drag") {
      const [[x0, y0], [x1, y1]] = rest.map((p) => p.split(",").map(Number));
      await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: x0, y: y0 });
      await send("Input.dispatchMouseEvent", { type: "mousePressed", x: x0, y: y0, button: "left", clickCount: 1 });
      for (let i = 1; i <= 10; i++) {
        const [x, y] = [x0 + ((x1 - x0) * i) / 10, y0 + ((y1 - y0) * i) / 10];
        await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y, button: "left", buttons: 1 });
        await sleep(30);
      }
      await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: x1, y: y1, button: "left", clickCount: 1 });
      await sleep(300);
    } else if (cmd === "wheel") {
      const [x, y] = rest[0].split(",").map(Number);
      await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
      await send("Input.dispatchMouseEvent", { type: "mouseWheel", x, y, deltaX: 0, deltaY: Number(rest[1]) });
      await sleep(300);
    } else if (cmd === "key") {
      const parts = arg.split("+");
      const key = parts.pop();
      const bits = { Alt: 1, Control: 2, Meta: 4, Shift: 8 };
      const modifiers = parts.reduce((m, p) => m | (bits[p] ?? 0), 0);
      const code = key.length === 1 ? `Key${key.toUpperCase()}` : key;
      const text = key.length === 1 && !(modifiers & 6) ? key : undefined;
      await send("Input.dispatchKeyEvent", { type: "keyDown", key, code, modifiers, text });
      await send("Input.dispatchKeyEvent", { type: "keyUp", key, code, modifiers });
      await sleep(300);
    } else if (cmd === "type") {
      await send("Input.insertText", { text: arg });
      await sleep(300);
    } else if (cmd === "upload") {
      uploads.push(arg);
      await send("Page.setInterceptFileChooserDialog", { enabled: true });
    } else if (cmd === "downloads") {
      await send("Browser.setDownloadBehavior", { behavior: "allow", downloadPath: arg });
    } else if (cmd === "reload") {
      await send("Page.reload");
      await sleep(4000);
    } else if (cmd === "wait") {
      await sleep(Number(arg));
    } else if (cmd === "shot") {
      await shot(arg);
    } else if (cmd === "eval") {
      const r = await send("Runtime.evaluate", { expression: arg, awaitPromise: true, returnByValue: true });
      console.log("eval:", JSON.stringify(r.result.value ?? r.result.description));
    } else {
      throw new Error(`unknown step ${step}`);
    }
  }
  await shot(out);
} finally {
  ws.close();
  chrome.kill();
}
