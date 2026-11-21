import { test as base, expect, type Page } from "@playwright/test";

/** Opens the UI on a mocked backend scenario (see ui/tests/mock.ts). */
export async function open(page: Page, scenario: "home" | "playing" | "engine-error" | "settings-error", opts: { settings?: boolean; onboarded?: boolean } = {}) {
  const onboarded = opts.onboarded ?? true;
  await page.addInitScript((done) => {
    if (done) localStorage.setItem("revlc.onboarded", "1");
    else localStorage.removeItem("revlc.onboarded");
  }, onboarded);
  const view = opts.settings ? "&view=settings" : "";
  await page.goto(`/tests/harness.html?scenario=${scenario}${view}`);
}

export const calls = (page: Page, cmd: string) => page.evaluate((c) => window.__mock.callsTo(c), cmd);

export const isMacProject = (name: string) => name === "macos";

/**
 * The app a layout test stands for, when this machine can stand for it:
 * the browser draws with this machine's fonts, so a test of what fits
 * where only means something on the matching OS. The macOS project on a
 * Mac, the Windows project on Windows, and the Windows project on Linux
 * (the Linux app uses the same layout); otherwise null, and the test skips.
 */
export function layoutHost(project: string): "macos" | "windows" | "linux" | null {
  if (project === "macos") return process.platform === "darwin" ? "macos" : null;
  if (process.platform === "win32") return "windows";
  if (process.platform === "linux") return "linux";
  return null;
}

/**
 * Fails a test on any uncaught exception, console error (Svelte's "update
 * depth exceeded" among them) or command the mock doesn't know.
 */
export const test = base.extend<{ watch: void }>({
  watch: [
    async ({ page }, use) => {
      const errors: string[] = [];
      page.on("pageerror", (e) => errors.push(`exception: ${e.message}`));
      page.on("console", (m) => {
        if (m.type() === "error") errors.push(`console: ${m.text()}`);
      });
      await use();
      const unknown = await page.evaluate(() => window.__mock?.unknown ?? []).catch(() => []);
      expect(unknown, "commands the mock doesn't know").toEqual([]);
      expect(errors, "errors in the page").toEqual([]);
    },
    { auto: true },
  ],
});

export { expect };
