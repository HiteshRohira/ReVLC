import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { test, expect } from "@playwright/test";

// The mock (ui/tests/mock.ts) only stands in for the backend if it knows
// the same commands: every command the UI sends must be one the app has
// (generate_handler! in src-tauri/src/lib.rs), and the mock must answer it,
// by name. Read from the source, so no page is needed.

const read = (path: string) => readFileSync(path, "utf8");

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? files(path) : /\.(ts|svelte)$/.test(name) ? [path] : [];
  });
}

/** The app's commands: the names in generate_handler![…]. */
function appCommands() {
  const list = read("src-tauri/src/lib.rs").match(/generate_handler!\[([\s\S]*?)\]/)![1].replace(/\/\/.*$/gm, "");
  return new Set(list.split(",").map((c) => c.trim().split("::").pop()!).filter(Boolean));
}

/** What the UI sends: the command names passed to invoke() or #call(). */
function uiCommands() {
  const sent = new Set<string>();
  for (const file of files("ui/src")) {
    for (const m of read(file).matchAll(/(?:invoke|#call)(?:<[^>]*>)?\(([^,)]*)/g)) {
      for (const name of m[1].matchAll(/"([a-z_0-9]+)"/g)) sent.add(name[1]);
    }
  }
  return sent;
}

/** What the mock answers: its handlers and its no-answer list. */
function mockCommands() {
  const mock = read("ui/tests/mock.ts");
  const handlers = mock.match(/const handlers[^{]*\{([\s\S]*?)\n\};/)![1];
  const named = [...handlers.matchAll(/^ {2}([a-z_0-9]+):/gm)].map((m) => m[1]);
  const noAnswer = [...mock.match(/const NO_ANSWER = new Set\(\[([\s\S]*?)\]\)/)![1].matchAll(/"([a-z_0-9]+)"/g)].map((m) => m[1]);
  return { named, noAnswer };
}

test("the UI only sends commands the app has", ({}, info) => {
  test.skip(info.project.name !== "windows", "reads the source; once is enough");
  const app = appCommands();
  const sent = [...uiCommands()];
  expect(sent.length, "found the UI's commands").toBeGreaterThan(50);
  expect(sent.filter((c) => !app.has(c)), "sent by the UI but not in generate_handler!").toEqual([]);
});

test("the mock answers exactly the app's commands", ({}, info) => {
  test.skip(info.project.name !== "windows", "reads the source; once is enough");
  const app = appCommands();
  const { named, noAnswer } = mockCommands();
  const mocked = new Set([...named, ...noAnswer]);
  expect([...mocked].filter((c) => !app.has(c)), "in the mock but not in the app").toEqual([]);
  expect([...uiCommands()].filter((c) => !mocked.has(c)), "sent by the UI but not in the mock").toEqual([]);
  expect(named.filter((c) => noAnswer.includes(c)), "both answered and on the no-answer list").toEqual([]);
});
