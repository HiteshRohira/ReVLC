// Makes the 200k-entry scan-cap fixture: 000-before-cap.mp4, 200,005 empty
// files, zzz-after-cap.mp4. Added as a library, only the first video should
// appear. Needs Red 60.mp4 and Blue 90.mp4 from generate-media.ps1.
//
//   node scripts/qa/windows/generate-entry-cap.cjs
const fs = require("node:fs"), path = require("node:path");
const media = process.env.QA_MEDIA ?? path.join(process.env.USERPROFILE, "revlc-qa", "media");
const dir = path.join(media, "Scan entry cap");
fs.mkdirSync(dir, { recursive: true });
const start = Date.now();
fs.copyFileSync(path.join(media, "Red 60.mp4"), path.join(dir, "000-before-cap.mp4"));
for (let i = 0; i < 200005; i++) {
  fs.writeFileSync(path.join(dir, `entry-${String(i).padStart(6, "0")}.txt`), "");
}
fs.copyFileSync(path.join(media, "Blue 90.mp4"), path.join(dir, "zzz-after-cap.mp4"));
console.log(
  JSON.stringify({ folder: dir, entries: fs.readdirSync(dir).length, creationMs: Date.now() - start }),
);
