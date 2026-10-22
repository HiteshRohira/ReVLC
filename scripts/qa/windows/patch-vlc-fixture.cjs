// Patches a COPIED libvlc.dll so its version string reads 4.0.24, to check
// that ReVLC refuses non-3.x libVLC. Never run it on the bundled DLL.
//
//   node scripts/qa/windows/patch-vlc-fixture.cjs <copy>\libvlc.dll
const fs = require("node:fs");
const p = process.argv[2];
const b = fs.readFileSync(p);
const needle = Buffer.from("3.0.24 Vetinari");
let pos = b.indexOf(needle);
if (pos < 0) throw Error("Version string absent");
if (b.indexOf(needle, pos + 1) >= 0) throw Error("Version string ambiguous");
b[pos] = 52;
fs.writeFileSync(p, b);
console.log(
  "Test fixture DLL reports 4.0.24 Vetinari; patched only copied DLL version string at offset " + pos,
);
