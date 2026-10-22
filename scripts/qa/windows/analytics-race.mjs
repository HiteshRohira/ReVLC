// Analytics race check: holds a batch in flight through a local proxy that
// never forwards (nothing reaches PostHog), turns analytics off, lets the
// send fail, turns it back on and checks the batch isn't retried.
// Run from the repo root after a debug build:
//
//   node scripts/qa/windows/analytics-race.mjs
import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import { spawn } from "node:child_process";
const root = process.env.QA_RUN;
if (!root) throw Error("Set QA_RUN to the run folder");
const repo = process.cwd();
const out = fs.openSync(path.join(root, "logs", "analytics-race-app-out.log"), "w");
const err = fs.openSync(path.join(root, "logs", "analytics-race-app-err.log"), "w");
const logFile = path.join(root, "logs", "analytics-race.jsonl");
const log = (name, data = {}) => {
  const e = { at: new Date().toISOString(), name, ...data };
  fs.appendFileSync(logFile, JSON.stringify(e) + "\n");
  console.log(JSON.stringify(e));
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sockets = [];
let requests = 0;
const proxy = net.createServer((socket) => {
  sockets.push(socket);
  socket.once("data", (buf) => {
    requests++;
    log("proxy CONNECT held", { requests, request: buf.toString().split("\r\n")[0] });
  });
});
await new Promise((r) => proxy.listen(9236, "127.0.0.1", r));
const app = spawn(
  path.join(process.env.CARGO_TARGET_DIR ?? path.join(repo, "src-tauri", "target"), "debug", "revlc.exe"),
  [],
  {
    cwd: repo,
    windowsHide: true,
    env: {
      ...process.env,
      REVLC_VLC_ARGS: "--no-audio",
      REVLC_ANALYTICS_SEND: "1",
      HTTPS_PROXY: "http://127.0.0.1:9236",
      HTTP_PROXY: "http://127.0.0.1:9236",
      NO_PROXY: "127.0.0.1,localhost",
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9223 --remote-allow-origins=*",
    },
    stdio: ["ignore", out, err],
  },
);
log("debug launched with isolated proxy", { pid: app.pid });
let ws;
try {
  let targets;
  for (let i = 0; i < 40; i++) {
    try {
      targets = await (await fetch("http://127.0.0.1:9223/json")).json();
      if (targets.length) break;
    } catch {}
    await sleep(250);
  }
  ws = new WebSocket(targets.find((t) => t.type === "page").webSocketDebuggerUrl);
  await new Promise((r, j) => {
    ws.onopen = r;
    ws.onerror = j;
  });
  let id = 0;
  const pending = new Map();
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (pending.has(m.id)) {
      const p = pending.get(m.id);
      pending.delete(m.id);
      clearTimeout(p.timer);
      p.resolve(m.result);
    }
  };
  const evaluate = (expression) =>
    new Promise((resolve, reject) => {
      const n = ++id;
      const timer = setTimeout(() => reject(Error("CDP timeout")), 15000);
      pending.set(n, {
        resolve: (r) => r.exceptionDetails ? reject(Error(r.exceptionDetails.text)) : resolve(r.result.value),
        timer,
      });
      ws.send(
        JSON.stringify({
          id: n,
          method: "Runtime.evaluate",
          params: { expression, returnByValue: true, awaitPromise: true },
        }),
      );
    });
  const setAnalytics = (enabled) =>
    evaluate(
      `window.__TAURI_INTERNALS__.invoke('settings_get').then(x=>{x.settings.analytics=${enabled};return window.__TAURI_INTERNALS__.invoke('settings_set',{settings:x.settings})})`,
    );
  const start = Date.now();
  while (requests === 0 && Date.now() - start < 45000) await sleep(250);
  if (!requests) throw Error("No analytics proxy request observed");
  await setAnalytics(false);
  log("analytics disabled while batch in flight");
  const failStart = Date.now();
  while (
    !fs.readFileSync(path.join(root, "logs", "analytics-race-app-err.log"), "utf8").includes(
      "analytics not sent",
    ) && Date.now() - failStart < 20000
  ) await sleep(250);
  const failed = fs.readFileSync(path.join(root, "logs", "analytics-race-app-err.log"), "utf8").includes(
    "analytics not sent",
  );
  log("held batch failure observed", { failed, requests });
  if (!failed) throw Error("Could not confirm send failure");
  await setAnalytics(true);
  const before = requests;
  log("analytics enabled again without new events", { requests: before });
  await sleep(35000);
  log("requeue regression result", {
    pass: requests === before,
    connectionsBefore: before,
    connectionsAfter: requests,
  });
  await setAnalytics(false);
  await evaluate('window.__TAURI_INTERNALS__.invoke("app_menu_item",{id:"quit"});null');
} catch (e) {
  log("race test failed", { error: String(e) });
  process.exitCode = 1;
} finally {
  ws?.close();
  for (const s of sockets) s.destroy();
  proxy.close();
}
