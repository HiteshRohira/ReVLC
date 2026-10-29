import { test, expect, open, isMacProject, layoutHost } from "./fixtures";

test("Home lists what to continue watching", async ({ page }) => {
  await open(page, "home");
  await expect(page.getByRole("heading", { name: "Continue watching" })).toBeVisible();
  await expect(page.getByText("Big Buck Bunny").first()).toBeVisible();
  await expect(page.getByRole("button", { name: "Remove Big Buck Bunny from history" })).toBeVisible();
});

test("removing a history entry asks the backend", async ({ page }) => {
  await open(page, "home");
  await page.getByRole("button", { name: "Remove Big Buck Bunny from history" }).click();
  await expect.poll(() => page.evaluate(() => window.__mock.callsTo("history_remove"))).toHaveLength(1);
});

test("the welcome screen shows on first launch, and Get started leads Home", async ({ page }) => {
  await open(page, "home", { onboarded: false });
  await expect(page.getByRole("heading", { name: "Welcome to ReVLC" })).toBeVisible();
  await page.getByRole("button", { name: "Get started" }).click();
  await expect(page.getByRole("heading", { name: "Home", exact: true })).toBeVisible();
});

test("the welcome screen fits a minimum-size window", async ({ page }, info) => {
  const host = layoutHost(info.project.name);
  test.skip(!host, "this machine's fonts don't match the project's OS");
  // With Linux's fonts the note fits, as it does in the Linux app.
  test.fail(host !== "linux", "W7: the usage statistics note runs below a 720×450 window");
  await page.setViewportSize({ width: 720, height: 450 });
  await open(page, "home", { onboarded: false });
  const note = page.getByText("ReVLC sends usage statistics");
  await expect(note).toBeVisible();
  const box = await note.boundingBox();
  expect(box!.y + box!.height).toBeLessThanOrEqual(450);
});

test("an engine failure is explained on Home", async ({ page }) => {
  await open(page, "engine-error");
  await expect(page.getByRole("heading", { name: "The playback engine couldn't start" })).toBeVisible();
  await expect(page.getByText("LoadLibraryExW failed")).toBeVisible();
});

test("Settings says why it can't load instead of Loading… forever", async ({ page }) => {
  await open(page, "settings-error", { settings: true });
  await expect(page.getByRole("alert")).toContainText("Settings are unavailable");
  await expect(page.getByText("Loading…")).toHaveCount(0);
});

test("Windows and Linux show the title-bar menus on Home", async ({ page }, info) => {
  test.skip(isMacProject(info.project.name), "macOS uses the menu bar");
  await open(page, "home");
  for (const name of ["Media", "Playback", "Audio", "Video", "Subtitles", "Tools", "Help"]) {
    await expect(page.getByRole("button", { name, exact: true }).first()).toBeVisible();
  }
});
