import { test, expect, open, calls, isMacProject } from "./fixtures";
import type { Page } from "@playwright/test";

const lastCall = async (page: Page, cmd: string) => (await calls(page, cmd)).at(-1);

test.beforeEach(async ({ page }) => {
  await open(page, "playing");
  await expect(page.getByRole("button", { name: "Pause" })).toBeVisible();
});

test("the Pause button pauses, and flips to Play", async ({ page }) => {
  await page.getByRole("button", { name: "Pause" }).click();
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible();
  expect(await calls(page, "toggle_pause")).toHaveLength(1);
});

test("keyboard shortcuts send the right commands", async ({ page }) => {
  const steps: [string, string, unknown][] = [
    ["Space", "toggle_pause", undefined],
    ["ArrowRight", "seek_by", { deltaMs: 5000 }],
    ["ArrowLeft", "seek_by", { deltaMs: -5000 }],
    ["Shift+ArrowRight", "seek_by", { deltaMs: 30000 }],
    ["ArrowUp", "set_volume", { volume: 105 }],
    ["m", "set_muted", { muted: true }],
    ["5", "seek", { ms: 298000 }],
    ["]", "set_rate", { rate: 1.25 }],
  ];
  for (const [key, cmd, args] of steps) {
    const before = (await calls(page, cmd)).length;
    await page.keyboard.press(key);
    await expect.poll(async () => (await calls(page, cmd)).length, { message: `${key} → ${cmd}` }).toBe(before + 1);
    if (args !== undefined) expect(await lastCall(page, cmd), `${key} → ${cmd}`).toEqual(args);
  }
});

test("holding Space toggles once; holding → keeps seeking", async ({ page }) => {
  const press = (key: string, code: string, repeat: boolean) =>
    page.evaluate(([k, c, r]) => document.body.dispatchEvent(new KeyboardEvent("keydown", { key: k as string, code: c as string, repeat: r as boolean, bubbles: true })), [key, code, repeat]);
  await press(" ", "Space", false);
  await press(" ", "Space", true);
  await press(" ", "Space", true);
  expect(await calls(page, "toggle_pause")).toHaveLength(1);
  await press("ArrowRight", "ArrowRight", false);
  await press("ArrowRight", "ArrowRight", true);
  await press("ArrowRight", "ArrowRight", true);
  expect(await calls(page, "seek_by")).toHaveLength(3);
});

test("the gear menu opens and closes many times without breaking", async ({ page }) => {
  for (let i = 0; i < 10; i++) {
    await page.getByRole("button", { name: "Settings" }).click();
    await expect(page.getByRole("menu", { name: "Settings" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu", { name: "Settings" })).toHaveCount(0);
  }
  // Still responsive: a state change still shows.
  await page.evaluate(() => window.__mock.setState({ time: 61_000 }));
  await expect(page.getByRole("button", { name: /^1:01/ })).toBeVisible();
});

test("media information keeps focus inside, and Escape gives it back", async ({ page }, info) => {
  // MediaInfoDialog only steps in at the first and last button and lets the
  // browser move focus otherwise; WebKit leaves buttons out of the Tab order
  // (unless the Mac's keyboard navigation setting is on), so its Shift+Tab
  // jumps past them to the seek bar behind the dialog.
  // WebKit takes that behaviour from macOS, so it only shows on a Mac.
  test.fail(isMacProject(info.project.name) && process.platform === "darwin",
    "found by this suite: on WebKit, Shift+Tab leaves Media information for the seek bar");
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: /^Media information/ }).click();
  const dialog = page.getByRole("dialog", { name: "Media information" });
  await expect(dialog).toBeVisible();
  for (const key of ["Tab", "Shift+Tab"]) {
    for (let i = 0; i < 15; i++) {
      await page.keyboard.press(key);
      // Never on a control behind the dialog. (WebKit's Tab skips buttons
      // unless the Mac's keyboard navigation setting is on, so focus can
      // rest on the page itself there.)
      const where = await page.evaluate(() => {
        const el = document.activeElement;
        return !el || el === document.body ? "page" : el.closest("[role=dialog]") ? "dialog" : el.outerHTML.slice(0, 80);
      });
      expect(["dialog", "page"], `${key} #${i + 1} reached ${where}`).toContain(where);
    }
  }
  await page.getByRole("tab", { name: "Codec" }).click();
  await expect(dialog).toContainText("H.264");
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});

test("the shortcut list names this platform's keys", async ({ page }, info) => {
  await page.keyboard.press("?");
  const help = page.getByRole("dialog", { name: "Keyboard shortcuts" });
  await expect(help).toBeVisible();
  const text = await help.innerText();
  if (isMacProject(info.project.name)) {
    expect(text).toContain("⌘");
    expect(text).toContain("Return");
  } else {
    expect(text).toContain("Ctrl+");
    expect(text).toContain("Enter");
    expect(text).not.toContain("Return");
    expect(text).not.toContain("⌘");
  }
});

test("the title-bar menus are reachable while playing", async ({ page }, info) => {
  test.skip(isMacProject(info.project.name), "macOS uses the menu bar");
  test.fail(true, "W6: the Windows and Linux menus only show on Home");
  await expect(page.getByRole("button", { name: "Playback", exact: true })).toBeVisible({ timeout: 2000 });
});

test("a resume offer continues from where it says", async ({ page }) => {
  await page.evaluate(() => window.__mock.emit("player://resume-prompt", { uri: "/media/Big Buck Bunny.mp4", ms: 120_000 }));
  await expect(page.getByText("Continue from 2:00?")).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect.poll(() => calls(page, "resume_at")).toEqual([{ ms: 120_000 }]);
  await expect(page.getByText("Continue from")).toHaveCount(0);
});

test("errors from the backend show as notices", async ({ page }) => {
  await page.evaluate(() => window.__mock.emit("player://notice", { kind: "error", text: "Could not play Sintel.mkv" }));
  await expect(page.getByText("Could not play Sintel.mkv")).toBeVisible();
});

test("effects: a filter switch sends the filter", async ({ page }) => {
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: /^Effects and filters/ }).click();
  const panel = page.getByRole("complementary", { name: "Effects and filters" });
  await expect(panel).toBeVisible();
  for (const tab of ["Equaliser", "Audio", "Colour", "Video", "Overlays"]) await expect(panel.getByRole("tab", { name: tab })).toBeVisible();
  await panel.getByRole("tab", { name: "Video" }).click();
  await panel.getByRole("switch", { name: "Invert colours" }).click();
  await expect.poll(() => calls(page, "set_filter")).toContainEqual({ id: "invert", filter: { enabled: true, values: {} } });
});

test("Up next lists the queue and removes an item", async ({ page }) => {
  await page.getByRole("button", { name: "Up next" }).click();
  const queue = page.getByRole("complementary", { name: "Up next" });
  await expect(queue).toContainText("Sintel");
  await queue.getByRole("button", { name: "Remove Sintel.mkv" }).click();
  await expect.poll(() => calls(page, "playlist_remove")).toEqual([{ index: 1 }]);
});

test("every control has an accessible name", async ({ page }) => {
  const unnamed = await page.evaluate(() =>
    [...document.querySelectorAll<HTMLElement>("button, [role=button], input, [role=switch], [role=slider]")]
      .filter((el) => el.offsetParent !== null)
      .filter((el) => !(el.getAttribute("aria-label") || el.getAttribute("aria-labelledby") || el.textContent?.trim() || el.getAttribute("title")))
      .map((el) => el.outerHTML.slice(0, 120)),
  );
  expect(unnamed).toEqual([]);
});

test("the player fits a minimum-size window", async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 450 });
  for (const name of ["Pause", "Fullscreen", "Settings", "Up next", "Subtitles", "Mute"]) {
    const box = await page.getByRole("button", { name, exact: true }).boundingBox();
    expect(box, name).not.toBeNull();
    expect(box!.x + box!.width, `${name} right edge`).toBeLessThanOrEqual(720);
    expect(box!.y + box!.height, `${name} bottom edge`).toBeLessThanOrEqual(450);
  }
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(720);
});
