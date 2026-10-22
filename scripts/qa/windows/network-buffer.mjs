// Quit-while-buffering check: serves a fixture over HTTP, stops sending
// bytes after a seek so playback buffers, quits through the app menu and
// reads the saved position from history.json.
//
//   node scripts/qa/windows/network-buffer.mjs
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { connect, log, media, root, shot as desktopShot, sleep } from "./cdp-lib.mjs";

const { rpc, evaluate, invoke, wait, close } = await connect();
// Installed-build screenshots get +100 and an "installed" tag, e.g. 145-installed-…
const shot = (name) =>
  desktopShot(
    process.env.QA_BUILD === "installed"
      ? name.replace(/^(\d+)-/, (_, n) => Number(n) + 100 + "-installed-")
      : name,
  );
const file = path.join(media, "H264 AAC.mp4");
const size = fs.statSync(file).size;
let holding = false;
let requests = [];
const timers = new Set();
const server = http.createServer((req, res) => {
  const match = /bytes=(\d+)-(\d*)/.exec(req.headers.range || "");
  const start = match ? Number(match[1]) : 0;
  const end = match && match[2] ? Number(match[2]) : size - 1;
  requests.push({ at: new Date().toISOString(), range: req.headers.range });
  res.writeHead(match ? 206 : 200, {
    "Content-Type": "video/mp4",
    "Content-Length": end - start + 1,
    "Accept-Ranges": "bytes",
    ...(match ? { "Content-Range": `bytes ${start}-${end}/${size}` } : {}),
  });
  let at = start;
  const fd = fs.openSync(file, "r");
  const tick = setInterval(() => {
    if (holding || res.writableNeedDrain) return;
    const n = Math.min(65536, end - at + 1);
    if (n <= 0) {
      clearInterval(tick);
      timers.delete(tick);
      fs.closeSync(fd);
      res.end();
      return;
    }
    const b = Buffer.alloc(n);
    fs.readSync(fd, b, 0, n, at);
    at += n;
    res.write(b);
  }, 30);
  timers.add(tick);
  res.on("close", () => {
    if (timers.delete(tick)) {
      clearInterval(tick);
      fs.closeSync(fd);
    }
  });
});
await new Promise((r) => server.listen(9235, "127.0.0.1", r));
try {
  await invoke("open", { uris: ["http://127.0.0.1:9235/QA-buffering.mp4"], play: true });
  const playing = await wait((s) => s.state?.time > 1000, 15000);
  log("network playback before hold", { snapshot: playing, requests });
  holding = true;
  await invoke("seek", { ms: 145000 });
  let stalled;
  for (let i = 0; i < 25; i++) {
    await sleep(200);
    stalled = await invoke("snapshot");
    if (stalled.state?.state === "buffering" && stalled.state?.time > 0) break;
  }
  shot("127-network-held-before-quit");
  log("network held while seek buffers", { snapshot: stalled, requests });
  await evaluate('window.__TAURI_INTERNALS__.invoke("app_menu_item",{id:"quit"});null');
  await sleep(1200);
  const history = JSON.parse(
    fs.readFileSync(path.join(process.env.APPDATA, "app.revlc", "history.json"), "utf8"),
  );
  log("history after buffering quit", {
    entry: history.find((e) => e.uri === "http://127.0.0.1:9235/QA-buffering.mp4"),
  });
} catch (e) {
  log("network test error", { error: String(e) });
} finally {
  for (const t of timers) clearInterval(t);
  server.closeAllConnections();
  server.close();
  close();
}
