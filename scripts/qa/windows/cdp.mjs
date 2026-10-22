// Evaluates JavaScript in ReVLC's WebView over CDP and prints the result.
//
//   node scripts/qa/windows/cdp.mjs "document.body.innerText"
//   node scripts/qa/windows/cdp.mjs --file expression.js
//   node scripts/qa/windows/cdp.mjs --capture 12-what-it-shows   (WebView-only PNG)
//
// See cdp-lib.mjs for the env it needs.
import { readFileSync } from "node:fs";
import { connect } from "./cdp-lib.mjs";

const [flag, arg] = process.argv.slice(2);
const cdp = await connect();
const timeout = setTimeout(() => {
  console.error("CDP timeout");
  process.exit(2);
}, 30000);
try {
  if (flag === "--capture") {
    console.log(await cdp.capture(arg));
  } else {
    const expression = flag === "--file" ? readFileSync(arg, "utf8") : process.argv.slice(2).join(" ");
    console.log(JSON.stringify(await cdp.evaluate(expression), null, 2));
  }
} finally {
  clearTimeout(timeout);
  cdp.close();
}
