// Scripted Windows checks used in the 2026-10-22 audit, run against a live
// app over CDP. Results go to $QA_RUN/logs/native-tests.jsonl.
//
//   node scripts/qa/windows/native-tests.mjs codecs|hls|library|heldkeys|resume|…
//
// Suites match that audit's change list and the fixtures from
// generate-media.ps1; adapt them to the branch under test.
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
const action = process.argv[2];
try {
  if (action === "codecs" || action === "hls") {
    for (
      const [i, file, video] of [
        [45, "H264 AAC.mp4", true],
        [46, "HEVC AAC.mkv", true],
        [47, "VP9 Opus.webm", true],
        [48, "MPEG4 MP3.avi", true],
        [49, "Audio WAV.wav", false],
        [50, "Audio FLAC.flac", false],
        [51, "Audio MP3.mp3", false],
        [52, "Paths with spaces/Unicode café हिन्दी 日本語 🎬.mp4", true],
        [53, "HLS TS/stream.m3u8", true],
        [54, "HLS TS/master.m3u8", true],
        [55, "HLS fMP4/stream.m3u8", true],
        [56, "HLS fMP4/master.m3u8", true],
      ]
    ) {
      if (action === "hls" && !file.endsWith("m3u8")) continue;
      await invoke("stop");
      await invoke("open", { uris: [path.join(media, file)], play: true });
      const s = await wait((s) =>
        s.state?.state === "playing" && s.state?.time > 500 && (!video || s.state.hasVideo)
      );
      await sleep(300);
      shot(`${i}-${file.replaceAll(/[^a-zA-Z0-9]+/g, "-")}`);
      log("playback " + file, {
        pass: s.state?.state === "playing" && s.state?.time > 0 && (!video || s.state.hasVideo),
        snapshot: s,
      });
    }
    for (const [i, file] of [[57, "Corrupt.mp4"], [58, "Zero byte.mp4"], [59, "Unsupported.xyz"]]) {
      if (action === "hls") continue;
      await invoke("stop");
      let err = null;
      try {
        await invoke("open", { uris: [path.join(media, file)], play: true });
      } catch (e) {
        err = String(e);
      }
      await sleep(1500);
      const s = await invoke("snapshot");
      shot(`${i}-${file.replaceAll(/[^a-zA-Z0-9]+/g, "-")}`);
      log("invalid " + file, { error: err, snapshot: s, ui: await evaluate("document.body.innerText") });
    }
    await invoke("stop");
  }
  if (action === "library") {
    await invoke("stop");
    const start = Date.now();
    const junction = await invoke("library_add", { path: path.join(media, "Junction Loop") });
    log("junction loop", {
      elapsedMs: Date.now() - start,
      pass: junction.items.length === 1,
      items: junction.items,
    });
    const series = await invoke("library_add", { path: path.join(media, "Series") });
    await sleep(3000);
    await evaluate(
      '(async()=>{const m=await import("/src/lib/player.svelte.ts");await m.player.loadLibrary();})()',
    );
    await evaluate(
      'Array.from(document.querySelectorAll("button")).find(b=>b.innerText.trim().startsWith("Videos")).click()',
    );
    shot("60-library-series-and-junction");
    fs.mkdirSync(path.join(media, "Mutable"), { recursive: true });
    fs.copyFileSync(path.join(media, "H264 AAC.mp4"), path.join(media, "Mutable", "Mutable.mp4"));
    await invoke("library_add", { path: path.join(media, "Mutable") });
    await sleep(3000);
    const before = await invoke("library_list");
    const candidate = before.items.find((i) => i.uri.endsWith("Mutable.mp4"));
    let hashBefore = null;
    if (candidate?.thumb && fs.existsSync(candidate.thumb)) {
      hashBefore = fs.readFileSync(candidate.thumb).toString("base64");
    }
    fs.copyFileSync(path.join(media, "HEVC AAC.mkv"), path.join(media, "Mutable", "Mutable.mp4"));
    const cached = await invoke("library_list");
    log("library scan cached under 30s", {
      sameModified: candidate?.modified === cached.items.find((i) => i.uri.endsWith("Mutable.mp4"))?.modified,
      before: candidate,
      cached: cached.items.find((i) => i.uri.endsWith("Mutable.mp4")),
    });
    await invoke("library_rescan");
    await sleep(5000);
    const fresh = await invoke("library_list");
    const after = fresh.items.find((i) => i.uri.endsWith("Mutable.mp4"));
    let hashAfter = null;
    if (after?.thumb && fs.existsSync(after.thumb)) {
      hashAfter = fs.readFileSync(after.thumb).toString("base64");
    }
    log("modified media metadata thumbnails", {
      before: candidate,
      after,
      thumbnailChanged: hashBefore !== hashAfter,
    });
    fs.copyFileSync(path.join(media, "VP9 Opus.webm"), path.join(media, "Mutable", "TTL new.webm"));
    const early = await invoke("library_list");
    log("TTL early cache", { newItemVisible: early.items.some((i) => i.uri.endsWith("TTL new.webm")) });
    await sleep(31000);
    const late = await invoke("library_list");
    log("TTL late rescan", { newItemVisible: late.items.some((i) => i.uri.endsWith("TTL new.webm")) });
    await evaluate(
      '(async()=>{const m=await import("/src/lib/player.svelte.ts");await m.player.loadLibrary(false);})()',
    );
    shot("61-library-refreshed-metadata");
  }
  if (action === "thumbnail") {
    await invoke("stop");
    const folder = path.join(media, "Thumbnail replacement");
    fs.mkdirSync(folder, { recursive: true });
    const file = path.join(folder, "Replacement.mp4");
    fs.copyFileSync(path.join(media, "Red 60.mp4"), file);
    fs.utimesSync(file, new Date(), new Date());
    await invoke("library_add", { path: folder });
    await sleep(5000);
    let list = await invoke("library_list");
    const before = list.items.find((i) => i.uri === file);
    const hashBefore = before?.thumb ? fs.readFileSync(before.thumb).toString("base64") : null;
    await evaluate(
      '(async()=>{const m=await import("/src/lib/player.svelte.ts");await m.player.loadLibrary();})()',
    );
    await evaluate(
      'Array.from(document.querySelectorAll("button")).find(b=>b.innerText.trim().startsWith("Videos")).click()',
    );
    await evaluate(
      'Array.from(document.querySelectorAll("h2")).find(x=>x.innerText.includes("Thumbnail replacement"))?.scrollIntoView({block:"start"})',
    );
    shot("72-thumbnail-red-original");
    fs.copyFileSync(path.join(media, "Blue 90.mp4"), file);
    const older = new Date(Date.now() - 3600000);
    fs.utimesSync(file, older, older);
    await invoke("library_rescan");
    await sleep(4000);
    list = await invoke("library_list");
    const after = list.items.find((i) => i.uri === file);
    const hashAfter = after?.thumb ? fs.readFileSync(after.thumb).toString("base64") : null;
    await evaluate(
      '(async()=>{const m=await import("/src/lib/player.svelte.ts");await m.player.loadLibrary();})()',
    );
    shot("73-thumbnail-stale-red-for-blue-file");
    log("older-timestamp replacement stale thumbnail", {
      before,
      after,
      sameThumbnail: hashBefore === hashAfter,
    });
    await invoke("open", { uris: [file], play: true });
    await wait((s) => s.state?.hasVideo && s.state?.time > 500);
    shot("74-replacement-actual-video-is-blue");
    await invoke("stop");
  }
  if (action === "cap") {
    await invoke("stop");
    const start = Date.now();
    const list = await invoke("library_add", { path: path.join(media, "Scan entry cap") });
    const items = list.items.filter((i) => i.root === path.join(media, "Scan entry cap"));
    log("200k-entry cap", {
      elapsedMs: Date.now() - start,
      fixtureEntries: fs.readdirSync(path.join(media, "Scan entry cap")).length,
      items,
      pass: items.length === 1 && items[0].uri.endsWith("000-before-cap.mp4"),
    });
    shot("93-scan-entry-cap-completed");
  }
  if (action === "focus") {
    await invoke("stop");
    await evaluate(
      'Array.from(document.querySelectorAll("button")).find(b=>b.innerText.trim()==="Open URL").click()',
    );
    await sleep(250);
    const before = await evaluate(
      '({tag:document.activeElement.tagName,label:document.activeElement.getAttribute("aria-label"),inside:!!document.activeElement.closest("[role=dialog]")})',
    );
    shot("125-network-dialog-keyboard-focus");
    await rpc("Input.dispatchKeyEvent", {
      type: "rawKeyDown",
      key: "Tab",
      code: "Tab",
      windowsVirtualKeyCode: 9,
      modifiers: 8,
    });
    await rpc("Input.dispatchKeyEvent", {
      type: "keyUp",
      key: "Tab",
      code: "Tab",
      windowsVirtualKeyCode: 9,
      modifiers: 8,
    });
    await sleep(200);
    shot("126-shift-tab-escapes-modal");
    const after = await evaluate(
      '({tag:document.activeElement.tagName,label:document.activeElement.getAttribute("aria-label"),text:document.activeElement.innerText,inside:!!document.activeElement.closest("[role=dialog]"),ariaModal:document.querySelector("[role=dialog]")?.getAttribute("aria-modal")})',
    );
    log("modal keyboard focus escapes", { before, after });
    await evaluate(
      'Array.from(document.querySelectorAll("button")).find(b=>b.innerText.trim()==="Cancel").click()',
    );
  }
  if (action === "general") {
    let longDir = path.join(media, "Long Paths");
    for (let i = 0; i < 6; i++) {
      longDir = path.join(longDir, `Level${i}-abcdefghijklmnopqrstuvwxyz0123456789`);
    }
    const longFile = path.join(longDir, "Long path unicode café 日本語 blue video.mp4");
    await invoke("open", { uris: [longFile], play: true });
    const ls = await wait((s) => s.state?.time > 500);
    shot("132-long-path-346-characters");
    log("long unicode path", {
      pathLength: longFile.length,
      fileExists: fs.existsSync(longFile),
      pass: ls.state?.state === "playing" && ls.state?.hasVideo,
      snapshot: ls,
      ui: await evaluate("document.body.innerText"),
    });
    await invoke("playlist_clear");
    const files = ["Red 60.mp4", "Blue 90.mp4", "Long Resume.mp4"].map((f) => path.join(media, f));
    await invoke("open", { uris: files, play: true });
    await wait((s) => s.state?.time > 500);
    const orig = (await invoke("snapshot")).playlist;
    await invoke("playlist_sort", { by: "reverse" });
    const sorted = (await invoke("snapshot")).playlist;
    await invoke("playlist_move", { from: 0, to: 1 });
    const moved = (await invoke("snapshot")).playlist;
    await invoke("set_repeat", { repeat: "one" });
    await invoke("set_shuffle", { shuffle: true });
    log("queue sort reorder repeat shuffle", {
      orig,
      sorted,
      moved,
      pass: sorted.items[sorted.current].uri === orig.items[orig.current].uri,
      snapshot: await invoke("snapshot"),
    });
    await evaluate(
      'Array.from(document.querySelectorAll("button")).find(b=>b.getAttribute("aria-label")==="Up next").click()',
    );
    shot("133-queue-reordered-repeat-shuffle");
    await invoke("set_shuffle", { shuffle: false });
    await invoke("set_repeat", { repeat: "off" });
    for (const ext of ["m3u8", "pls", "xspf"]) {
      const list = path.join(root, `QA playlist with spaces.${ext}`);
      await invoke("playlist_save", { path: list });
      await invoke("open", { uris: [list], play: true });
      const s = await wait((s) => s.state?.time > 0);
      log("saved playlist round trip " + ext, { file: list, snapshot: s });
    }
    await invoke("seek", { ms: 12000 });
    await sleep(250);
    await invoke("bookmark_add", { name: "QA Unicode café 日本語 bookmark" });
    let s = await invoke("snapshot");
    const bookmark = s.bookmarks[0];
    await invoke("bookmark_rename", { time: bookmark.time, name: "Renamed QA bookmark" });
    await invoke("set_ab_loop", { a: 10000, b: 15000 });
    await invoke("seek", { ms: 14000 });
    await sleep(1800);
    log("bookmark rename and AB loop", { snapshot: await invoke("snapshot") });
    shot("134-bookmark-and-ab-loop-state");
    await invoke("set_ab_loop", { a: null, b: null });
    await invoke("set_sleep_timer", { minutes: 15, afterItem: false });
    shot("135-sleep-timer-active");
    await invoke("set_sleep_timer", { minutes: null, afterItem: false });
    await invoke("bookmark_remove", { time: bookmark.time });
    await invoke("playlist_remove", { index: 0 });
    log("queue remove", { snapshot: await invoke("snapshot") });
    await invoke("stop");
    const bad = path.join(media, "Malformed thumbnails");
    fs.mkdirSync(bad, { recursive: true });
    fs.copyFileSync(path.join(media, "Red 60.mp4"), path.join(bad, "a-valid.mp4"));
    fs.writeFileSync(path.join(bad, "b-corrupt.mp4"), "This is not valid media");
    fs.writeFileSync(path.join(bad, "c-unsupported.mp4"), "Random unsupported payload");
    fs.writeFileSync(path.join(bad, "d-zero.mp4"), "");
    fs.writeFileSync(path.join(bad, "e-unsupported.xyz"), "Unsupported");
    const before = execFileSync("powershell.exe", [
      "-NoProfile",
      "-Command",
      "(Get-Process revlc).PrivateMemorySize64",
    ], { encoding: "utf8" }).trim();
    await invoke("library_add", { path: bad });
    await sleep(22000);
    const list = await invoke("library_list");
    const after = execFileSync("powershell.exe", [
      "-NoProfile",
      "-Command",
      "(Get-Process revlc).PrivateMemorySize64",
    ], { encoding: "utf8" }).trim();
    log("malformed thumbnail smoke", {
      privateBytesBefore: before,
      privateBytesAfter: after,
      items: list.items.filter((i) => i.root === bad),
      snapshot: await invoke("snapshot"),
    });
    shot("136-malformed-thumbnail-smoke-survives");
  }
  if (action === "heldkeys") {
    await invoke("open", { uris: [path.join(media, "Long Resume.mp4")], play: true });
    await wait((s) => s.state?.time > 500);
    await invoke("seek", { ms: 10000 });
    await sleep(300);
    await invoke("toggle_pause");
    await sleep(200);
    const before = await invoke("snapshot");
    for (let i = 0; i < 11; i++) {
      await rpc("Input.dispatchKeyEvent", {
        type: "rawKeyDown",
        key: "ArrowRight",
        code: "ArrowRight",
        windowsVirtualKeyCode: 39,
        autoRepeat: i > 0,
      });
      await sleep(110);
    }
    await rpc("Input.dispatchKeyEvent", {
      type: "keyUp",
      key: "ArrowRight",
      code: "ArrowRight",
      windowsVirtualKeyCode: 39,
    });
    await sleep(400);
    const after = await invoke("snapshot");
    shot("148-held-seek-repeats-while-paused");
    log("held seek repeat", {
      before: before.state,
      after: after.state,
      pass: after.state.time - before.state.time > 40000,
    });
    for (let i = 0; i < 11; i++) {
      await rpc("Input.dispatchKeyEvent", {
        type: "rawKeyDown",
        key: " ",
        code: "Space",
        windowsVirtualKeyCode: 32,
        autoRepeat: i > 0,
      });
      await sleep(70);
    }
    await rpc("Input.dispatchKeyEvent", {
      type: "keyUp",
      key: " ",
      code: "Space",
      windowsVirtualKeyCode: 32,
    });
    await sleep(400);
    log("held space toggles once", { state: (await invoke("snapshot")).state });
    await invoke("stop");
  }
  if (action === "resume") {
    await invoke("history_remove", { uri: path.join(media, "H264 AAC.mp4") });
    let cfg = (await invoke("settings_get")).settings;
    cfg.resume = "always";
    await invoke("settings_set", { settings: cfg });
    await invoke("open", { uris: [path.join(media, "Long Resume.mp4")], play: true });
    await wait((s) => s.state?.time > 500);
    await invoke("seek", { ms: 120000 });
    await sleep(1200);
    const a = await invoke("snapshot");
    log("resume source before switch", { snapshot: a });
    await invoke("open", { uris: [path.join(media, "H264 AAC.mp4")], play: true });
    const b = await wait((s) => s.state?.time > 500);
    log("resume switch to different item", { snapshot: b });
    await invoke("open", { uris: [path.join(media, "Long Resume.mp4")], play: true });
    const a2 = await wait((s) => s.state?.time > 100000);
    shot("62-resume-always-long-file");
    log("resume return", { snapshot: a2 });
    await invoke("seek", { ms: 210000 });
    await sleep(11500);
    log("history playback progress on disk", {
      history: JSON.parse(
        fs.readFileSync(path.join(process.env.APPDATA, "app.revlc", "history.json"), "utf8"),
      ),
    });
    shot("63-resume-progress-before-exit");
  }
} catch (e) {
  log("suite failure", { action, error: String(e), stack: e.stack });
  process.exitCode = 1;
}
close();
