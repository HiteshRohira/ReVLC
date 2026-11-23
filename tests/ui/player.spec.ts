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

test("the shortcut list scrolls from the keyboard, not the player", async ({ page }) => {
  await page.keyboard.press("?");
  const help = page.getByRole("dialog", { name: "Keyboard shortcuts" });
  await expect(help).toBeFocused();
  const scrolled = () => help.evaluate((el) => el.scrollTop);
  expect(await help.evaluate((el) => el.scrollHeight > el.clientHeight), "the list is long enough to scroll").toBe(true);
  // The list has only just taken focus; under load the browser can drop an
  // arrow key that arrives in the same moment, so press until it scrolls.
  await expect(async () => {
    await page.keyboard.press("ArrowDown");
    expect(await scrolled()).toBeGreaterThan(0);
  }).toPass({ timeout: 5000 });
  await page.keyboard.press("End");
  await expect.poll(scrolled).toBe(await help.evaluate((el) => el.scrollHeight - el.clientHeight));
  expect(await calls(page, "set_volume"), "ArrowDown went to the player").toEqual([]);
  await page.keyboard.press("Escape");
  await expect(help).toHaveCount(0);
});

test("a stalled stream is announced as Buffering", async ({ page }) => {
  // The mock's clock stands still, which the player reads as a stall.
  const status = page.getByRole("status").filter({ hasText: /^(Buffering)?$/ });
  await expect(status).toHaveText("Buffering");
  // The same live region, emptied, so the next stall is announced too.
  await status.evaluate((el) => ((window as unknown as { region: Element }).region = el));
  await page.getByRole("button", { name: "Pause" }).click();
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible();
  expect(await page.evaluate(() => {
    const el = (window as unknown as { region: Element }).region;
    return { connected: el.isConnected, text: el.textContent };
  })).toEqual({ connected: true, text: "" });
});

test("the title-bar menus are reachable while playing", async ({ page }, info) => {
  test.skip(isMacProject(info.project.name), "macOS uses the menu bar");
  await expect(page.getByRole("button", { name: "Playback", exact: true })).toBeVisible({ timeout: 2000 });
  // They don't cover the back button or the title, and they work.
  const box = async (l: import("@playwright/test").Locator) => (await l.boundingBox())!;
  const back = await box(page.getByRole("button", { name: "Back to home" }));
  const first = await box(page.getByRole("button", { name: "Media", exact: true }));
  const last = await box(page.getByRole("button", { name: "Help", exact: true }));
  const title = await box(page.locator(".top .title"));
  expect(first.x).toBeGreaterThanOrEqual(back.x + back.width);
  expect(title.x).toBeGreaterThanOrEqual(last.x + last.width);
  await page.getByRole("button", { name: "Playback", exact: true }).click();
  await page.getByRole("menuitem", { name: /^Play \/ Pause/ }).click();
  await expect.poll(() => calls(page, "toggle_pause")).toHaveLength(1);
});

test("Space and Return press the button the keyboard is on, not play or pause", async ({ page }, info) => {
  // Tab from Next reaches Mute: Space is that button's. (On a Mac, WebKit's
  // plain Tab skips buttons.)
  await page.getByRole("button", { name: "Next", exact: true }).focus();
  await page.keyboard.press(isMacProject(info.project.name) && process.platform === "darwin" ? "Alt+Tab" : "Tab");
  await expect(page.getByRole("button", { name: "Mute" })).toBeFocused();
  await page.keyboard.press("Space");
  await expect.poll(() => calls(page, "set_muted")).toHaveLength(1);
  expect(await calls(page, "toggle_pause")).toHaveLength(0);
});

test("Space after clicking a button still pauses", async ({ page }) => {
  await page.getByRole("button", { name: "Mute" }).click();
  await page.keyboard.press("Space");
  await expect.poll(() => calls(page, "toggle_pause")).toHaveLength(1);
  expect(await calls(page, "set_muted")).toHaveLength(1);
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
    // In the player's own controls: the title bar has a Subtitles menu too.
    const box = await page.locator(".player").getByRole("button", { name, exact: true }).boundingBox();
    expect(box, name).not.toBeNull();
    expect(box!.x + box!.width, `${name} right edge`).toBeLessThanOrEqual(720);
    expect(box!.y + box!.height, `${name} bottom edge`).toBeLessThanOrEqual(450);
  }
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(720);
});

test("the window title ends on the newest file when replies come back out of order", async ({ page }) => {
  // Tauri runs each set_title on its own thread, so two quick ones can land
  // in either order. Found by the Cua suite on a slow Windows VM.
  const playlist = (current: number) => ({
    items: [{ uri: "/media/Big Buck Bunny.mp4", title: "Big Buck Bunny.mp4" }, { uri: "/media/Sintel.mkv", title: "Sintel.mkv" }],
    current, repeat: "off", shuffle: false,
  });
  await page.evaluate(() => (window.__mock.delays["plugin:window|set_title"] = [400]));
  await page.evaluate((p) => window.__mock.emit("player://playlist", p), playlist(1));
  await page.evaluate((p) => window.__mock.emit("player://playlist", p), playlist(0));
  const asked = async () => ((await calls(page, "plugin:window|set_title")).at(-1) as { value: string }).value;
  await expect.poll(asked).toContain("Big Buck Bunny");
  // Every reply in, then the window has the newest title, and titles were
  // sent one at a time.
  await page.waitForTimeout(600);
  expect(await page.evaluate(() => window.__mock.windowTitle())).toBe(await asked());
  expect(await page.evaluate(() => window.__mock.maxInFlight["plugin:window|set_title"])).toBe(1);
});

// Verified 16x9 RGB JPEG generated by Pillow (decodes cleanly in WebKit & Chromium).
const VALID_JPEG = Buffer.from(
  "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAUDBAQEAwUEBAQFBQUGBwwIBwcHBw8LCwkMEQ8SEhEPERETFhwXExQaFRERGCEYGh0dHx8fExciJCIeJBweHx7/2wBDAQUFBQcGBw4ICA4eFBEUHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh4eHh7/wAARCAAJABADASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwCvRRRX5of0Kf/Z",
  "base64",
);

test("seek bar hover renders in-memory preview over custom protocol and focuses unready slots", async ({ page }) => {
  await page.route("http://scrub.localhost/**", (route) =>
    route.fulfill({ status: 200, contentType: "image/jpeg", body: VALID_JPEG }),
  );
  await page.evaluate(() => {
    window.__mock.scrubPrepare = () => ({
      run: 7,
      prefix: "http://scrub.localhost/7/",
      step: 2000,
      ready: [0],
    });
  });

  // Switch item so SeekBar's effect schedules a fresh scrub_prepare with our mock handler.
  await page.evaluate(() =>
    window.__mock.emit("player://playlist", {
      items: [{ uri: "/media/ClipA.mp4", title: "ClipA.mp4" }],
      current: 0,
      repeat: "off",
      shuffle: false,
    }),
  );
  await expect.poll(async () => (await calls(page, "scrub_prepare")).length, { timeout: 6000 }).toBeGreaterThanOrEqual(1);

  const seek = page.getByRole("slider", { name: "Seek" });
  // Wake the auto-hiding HUD if it faded during the 2.5s prepare delay, then hover exact midpoint
  // (length = 596_000 ms, midpoint = 298_000 ms -> expected slot = 149).
  await page.mouse.move(100, 100);
  await seek.hover();
  await expect(page.getByRole("status", { name: "Making preview" })).toBeVisible();
  await expect.poll(async () => (await calls(page, "scrub_focus")).length).toBeGreaterThanOrEqual(1);
  const focus = (await lastCall(page, "scrub_focus")) as { from: number; to: number };
  expect(focus.from).toBeLessThanOrEqual(149);
  expect(focus.to).toBeGreaterThanOrEqual(149);

  // Deliver slot 149 via player://scrub -> "Making preview" is replaced by a real decoded JPEG.
  await page.evaluate(() => window.__mock.emit("player://scrub", { run: 7, k: 149 }));
  const preview = page.getByRole("img", { name: "Seek preview" });
  await expect(preview).toBeVisible();
  await expect(preview).toHaveAttribute("src", /^http:\/\/scrub\.localhost\/7\/149\.jpg\?v=\d+$/);
  await expect.poll(() => preview.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBe(16);
  await expect(page.getByRole("status", { name: "Making preview" })).toHaveCount(0);
});

test("early scrub events buffer during prepare and cancelled prepares are discarded", async ({ page }) => {
  await page.route("http://scrub.localhost/**", (route) =>
    route.fulfill({ status: 200, contentType: "image/jpeg", body: VALID_JPEG }),
  );
  // Install a manual deferred-promise controller so we control exact ordering with zero wall-clock races.
  await page.evaluate(() => {
    const w = window as unknown as {
      __resolvers: Record<number, (val: unknown) => void>;
      __mode: "defer" | "immediate";
      __nextRun: number;
    };
    w.__resolvers = {};
    w.__mode = "defer";
    w.__nextRun = 10;
    window.__mock.scrubPrepare = () => {
      const run = ++w.__nextRun;
      if (w.__mode === "immediate") {
        return { run, prefix: `http://scrub.localhost/${run}/`, step: 2000, ready: [149] };
      }
      return new Promise((resolve) => {
        w.__resolvers[run] = resolve as (val: unknown) => void;
      });
    };
  });

  // Phase A: Early event arrives while scrub_prepare (run 11) is still unresolved.
  await page.evaluate(() =>
    window.__mock.emit("player://playlist", {
      items: [{ uri: "/media/Early.mp4", title: "Early.mp4" }],
      current: 0,
      repeat: "off",
      shuffle: false,
    }),
  );
  await expect.poll(() => page.evaluate(() => 11 in (window as unknown as { __resolvers: Record<number, unknown> }).__resolvers), { timeout: 6000 }).toBe(true);

  const seek = page.getByRole("slider", { name: "Seek" });
  await page.mouse.move(100, 100);
  await seek.hover();
  // Emit slot 149 BEFORE run 11 resolves -> must buffer in #scrubPending, NOT render yet.
  await page.evaluate(() => window.__mock.emit("player://scrub", { run: 11, k: 149 }));
  await expect(page.getByRole("img", { name: "Seek preview" })).toHaveCount(0);

  // Now resolve run 11 with ready: [] -> buffered slot 149 immediately renders with naturalWidth > 0.
  await page.evaluate(() => {
    const r = (window as unknown as { __resolvers: Record<number, (v: unknown) => void> }).__resolvers[11];
    r({ run: 11, prefix: "http://scrub.localhost/11/", step: 2000, ready: [] });
  });
  const preview = page.getByRole("img", { name: "Seek preview" });
  await expect(preview).toBeVisible();
  await expect(preview).toHaveAttribute("src", /^http:\/\/scrub\.localhost\/11\/149\.jpg\?v=\d+$/);
  await expect.poll(() => preview.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBe(16);

  // Phase B: Switch to ClipB (deferred run 12), then switch to ClipC (immediate run 13) while
  // playback remains active. Resolving stale run 12 WITH ready: [149] + stale event must NOT
  // overwrite run 13's rendered preview.
  await page.evaluate(() =>
    window.__mock.emit("player://playlist", {
      items: [{ uri: "/media/ClipB.mp4", title: "ClipB.mp4" }],
      current: 0,
      repeat: "off",
      shuffle: false,
    }),
  );
  await expect.poll(() => page.evaluate(() => 12 in (window as unknown as { __resolvers: Record<number, unknown> }).__resolvers), { timeout: 6000 }).toBe(true);

  const cancelsBeforeSwitch = (await calls(page, "scrub_cancel")).length;
  await page.evaluate(() => {
    (window as unknown as { __mode: string }).__mode = "immediate";
    window.__mock.emit("player://playlist", {
      items: [{ uri: "/media/ClipC.mp4", title: "ClipC.mp4" }],
      current: 0,
      repeat: "off",
      shuffle: false,
    });
  });
  await expect.poll(async () => (await calls(page, "scrub_cancel")).length).toBeGreaterThan(cancelsBeforeSwitch);
  await expect.poll(async () => (await calls(page, "scrub_prepare")).length, { timeout: 6000 }).toBeGreaterThanOrEqual(3);

  await page.mouse.move(100, 100);
  await seek.hover();
  await expect(preview).toHaveAttribute("src", /^http:\/\/scrub\.localhost\/13\/149\.jpg\?v=\d+$/);

  // Release stale run 12 with ready: [149] AND send a stale run 12 frame event -> run 13 must stay rendered.
  await page.evaluate(() => {
    const r = (window as unknown as { __resolvers: Record<number, (v: unknown) => void> }).__resolvers[12];
    r({ run: 12, prefix: "http://scrub.localhost/12/", step: 2000, ready: [149] });
    window.__mock.emit("player://scrub", { run: 12, k: 149 });
  });
  await page.waitForTimeout(100);
  await expect(preview).toHaveAttribute("src", /^http:\/\/scrub\.localhost\/13\/149\.jpg\?v=\d+$/);
  await expect.poll(() => preview.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBe(16);
});

test("the gear menu's Deinterlace toggle follows Settings until it's used", async ({ page }) => {
  const toggle = page.getByRole("menuitemcheckbox", { name: "Deinterlace" });
  const settingsWith = (patch: Record<string, unknown>) => page.evaluate((p) => window.__mock.setSettings(p), patch);
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await settingsWith({ deinterlace: 1, deinterlaceMode: "blend" });
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  expect(await lastCall(page, "set_deinterlace")).toEqual({ mode: null });
  await toggle.click();
  expect(await lastCall(page, "set_deinterlace")).toEqual({ mode: "blend" });
  // Settings changing it again takes over from the toggle.
  await settingsWith({ deinterlace: 0, deinterlaceMode: "blend" });
  await expect(toggle).toHaveAttribute("aria-checked", "false");
});
