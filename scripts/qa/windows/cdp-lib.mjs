// Shared helpers for driving ReVLC's WebView2 over the Chrome DevTools
// Protocol during QA runs; see docs/audit-playbook.md.
//
// The app must be started with
//   WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223 --remote-allow-origins=*"
// Env: QA_RUN (run folder, required), QA_MEDIA (fixtures, default
// %USERPROFILE%\revlc-qa\media), QA_CDP_PORT (default 9223),
// QA_WINDOW=settings to target the Settings window instead of the main one,
// QA_BUILD (label written into the test log, e.g. debug or installed).
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

export const here = path.dirname(fileURLToPath(import.meta.url));
if (!process.env.QA_RUN) throw Error("Set QA_RUN to the run folder");
export const root = process.env.QA_RUN;
export const media = process.env.QA_MEDIA ??
  path.join(process.env.USERPROFILE, "revlc-qa", "media");
export const port = process.env.QA_CDP_PORT ?? 9223;
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** The main window's page target, or Settings with QA_WINDOW=settings.
 * Never falls back to the first target: that may be the other window. */
export async function findTarget(window = process.env.QA_WINDOW) {
  const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
  const target = targets.find((t) =>
    t.type === "page" &&
    (window === "settings") === t.url.includes("view=settings")
  );
  if (!target) throw Error(`No WebView2 target for ${window ?? "main"}`);
  return target;
}

export async function connect(window) {
  const ws = new WebSocket((await findTarget(window)).webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = reject;
  });
  let serial = 0;
  const pending = new Map();
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    const p = pending.get(m.id);
    if (!p) return;
    pending.delete(m.id);
    clearTimeout(p.timer);
    m.error ? p.reject(m.error) : p.resolve(m.result);
  };
  const rpc = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++serial;
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(Error("Timeout " + method));
      }, 20000);
      pending.set(id, { resolve, reject, timer });
      ws.send(JSON.stringify({ id, method, params }));
    });
  // Return plain values: an object holding an un-awaited promise never resolves.
  const evaluate = async (expression) => {
    const r = await rpc("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
      userGesture: true,
    });
    if (r.exceptionDetails) {
      throw Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
    }
    return r.result.value;
  };
  const invoke = (name, args = {}) =>
    evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(name)},${JSON.stringify(args)})`);
  /** Polls the backend snapshot until pred holds or the timeout passes. */
  const wait = async (pred, timeout = 8000) => {
    const start = Date.now();
    let s;
    do {
      s = await invoke("snapshot");
      if (pred(s)) return s;
      await sleep(250);
    } while (Date.now() - start < timeout);
    return s;
  };
  /** Captures the WebView only: immune to covering windows, but without the
   * native video surface, so use shot() when frames or subtitles matter. */
  const capture = async (name) => {
    const png = await rpc("Page.captureScreenshot", { format: "png", fromSurface: true });
    const file = path.join(root, "screenshots", name + ".png");
    fs.writeFileSync(file, Buffer.from(png.data, "base64"));
    return file;
  };
  return { ws, rpc, evaluate, invoke, wait, capture, close: () => ws.close() };
}

/** Appends one result to logs/native-tests.jsonl; the report's PASS/FAIL table
 * comes from this file. */
export function log(name, result) {
  const entry = { at: new Date().toISOString(), build: process.env.QA_BUILD ?? "debug", name, ...result };
  fs.appendFileSync(path.join(root, "logs", "native-tests.jsonl"), JSON.stringify(entry) + "\n");
  console.log(JSON.stringify(entry));
}

/** Desktop screenshot through qa-desktop.ps1 (shows video frames). */
export function shot(name) {
  const ps1 = path.join(here, "qa-desktop.ps1");
  return execFileSync("powershell.exe", [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-Command",
    `. '${ps1}'; Shot '${name}'`,
  ], { encoding: "utf8" });
}
