// Cuts the full HKCU exports in $QA_RUN\backup (hkcu-classes.reg,
// hkcu-uninstall.reg, hkcu-registeredapps.reg, made with `reg export` before
// installing a test build) down to ReVLC's own keys, ready for `reg import`.
// Run from the repo root (it reads src-tauri/windows/hooks.nsh).
//
//   node scripts/qa/windows/prepare-registry-restore.cjs
const fs = require("node:fs"), path = require("node:path");
const root = process.env.QA_RUN, backup = path.join(root, "backup");
const read = (p) => fs.readFileSync(path.join(backup, p), "utf16le").replace(/^\uFEFF/, "");
const sections = (text) =>
  [...text.matchAll(/\[(HKEY[^\]]+)\]\r?\n([\s\S]*?)(?=\r?\n\[HKEY|$)/g)].map((m) => ({
    key: m[1],
    body: m[2],
  }));
const write = (name, secs) => {
  fs.writeFileSync(
    path.join(backup, name),
    "\uFEFFWindows Registry Editor Version 5.00\r\n\r\n" +
      secs.map((s) => `[${s.key}]\r\n${s.body.trimEnd()}\r\n`).join("\r\n"),
    "utf16le",
  );
  console.log(`${name}: ${secs.length} keys`);
};
const hooks = fs.readFileSync("src-tauri/windows/hooks.nsh", "utf8");
const exts = new Set(
  [...hooks.matchAll(/!insertmacro \$\{MACRO\} "([^"]+)"/g)].map((m) => "." + m[1].toLowerCase()),
);
const prefix = "hkey_current_user\\software\\classes\\";
const selected = sections(read("hkcu-classes.reg")).filter((s) => {
  const k = s.key.toLowerCase();
  if (!k.startsWith(prefix)) return false;
  const rest = k.slice(prefix.length), base = rest.split("\\")[0];
  return exts.has(base) ||
    ["revlc.video", "revlc.audio", "revlc.playlist", "video", "audio", "playlist"].includes(base) ||
    rest === "applications\\revlc.exe" || rest.startsWith("applications\\revlc.exe\\");
});
write("restore-revlc-classes-only.reg", selected);
write(
  "restore-revlc-uninstall-only.reg",
  sections(read("hkcu-uninstall.reg")).filter((s) =>
    /^hkey_current_user\\software\\microsoft\\windows\\currentversion\\uninstall\\revlc(?:\\|$)/i.test(s.key)
  ),
);
const reg = sections(read("hkcu-registeredapps.reg")).find((s) =>
  s.key.toLowerCase() === "hkey_current_user\\software\\registeredapplications"
);
const line = reg?.body.split(/\r?\n/).find((l) => /^"ReVLC"=/i.test(l));
write("restore-revlc-registeredapp-only.reg", line ? [{ key: reg.key, body: line }] : []);
