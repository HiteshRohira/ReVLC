// Copies a fixture to a ~340-character Unicode path (check LongPathsEnabled
// first). Prints the path and its length.
//
//   node scripts/qa/windows/generate-longpath.cjs
const fs = require("node:fs"), path = require("node:path");
const root = process.env.QA_MEDIA ?? path.join(process.env.USERPROFILE, "revlc-qa", "media");
let dir = path.join(root, "Long Paths");
for (let i = 0; i < 6; i++) dir = path.join(dir, `Level${i}-abcdefghijklmnopqrstuvwxyz0123456789`);
fs.mkdirSync(dir, { recursive: true });
const target = path.join(dir, "Long path unicode café 日本語 blue video.mp4");
fs.copyFileSync(path.join(root, "Blue 90.mp4"), target);
console.log(
  JSON.stringify({
    path: target,
    length: target.length,
    exists: fs.existsSync(target),
    size: fs.statSync(target).size,
  }),
);
