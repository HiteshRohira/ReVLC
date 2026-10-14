// Merges the update manifests each platform's build wrote into one.
//
//   node scripts/merge-manifests.js site/latest.json build1/latest.json build2/latest.json
//
// Each build adds only its own platform (scripts/release-files.sh), so this
// takes every platform that has the current version and writes the result
// to the first path. Like release-files.sh, the manifest moves to the new
// version only once every platform in it has it. $NOTES sets the notes.
import { readFileSync, writeFileSync } from "node:fs";

const [out, ...builds] = process.argv.slice(2);
const version = JSON.parse(readFileSync("package.json", "utf8")).version;
const tag = `v${version}-alpha`;
const current = (entry) => entry.url.includes(`/${tag}/`);

const manifest = JSON.parse(readFileSync(out, "utf8"));
for (const path of builds) {
  for (const [platform, entry] of Object.entries(JSON.parse(readFileSync(path, "utf8")).platforms)) {
    if (current(entry)) manifest.platforms[platform] = entry;
  }
}

const waiting = Object.keys(manifest.platforms).filter((p) => !current(manifest.platforms[p]));
if (waiting.length === 0) {
  manifest.version = version;
  manifest.notes = process.env.NOTES || `See https://github.com/HiteshRohira/ReVLC/releases/tag/${tag}`;
  manifest.pub_date = new Date().toISOString().replace(/\.\d+Z$/, "Z");
  console.log(`${out} now offers ${version} on every platform`);
} else {
  console.log(`${out} stays at ${manifest.version}: ${waiting.join(", ")} still needs ${tag}`);
  process.exitCode = 1;
}
const { notes, pub_date, platforms } = manifest;
writeFileSync(out, JSON.stringify({ version: manifest.version, notes, pub_date, platforms }, null, 2) + "\n");
