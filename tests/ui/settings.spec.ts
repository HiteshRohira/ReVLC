import { test, expect, open, calls, isMacProject } from "./fixtures";

const TABS = ["General", "Video", "Audio", "Subtitles", "Input & Codecs", "Shortcuts"];

test.beforeEach(async ({ page }) => {
  await open(page, "home", { settings: true });
  await expect(page.getByRole("button", { name: "General" })).toBeVisible();
});

test("every tab loads", async ({ page }) => {
  for (const tab of TABS) {
    await page.getByRole("button", { name: tab, exact: true }).click();
    await expect(page.getByText("Loading…")).toHaveCount(0);
    await expect(page.locator("main")).not.toBeEmpty();
  }
});

test("a switch saves the setting", async ({ page }) => {
  await page.getByRole("switch", { name: "Pause when minimised" }).click();
  await expect.poll(async () => (await calls(page, "settings_set")).at(-1)).toMatchObject({ settings: { pauseWhenMinimized: true } });
});

test("a failed save says so", async ({ page }) => {
  await page.evaluate(() => (window.__mock.failing.settings_set = "the disk is full"));
  await page.getByRole("switch", { name: "Pause when minimised" }).click();
  await expect(page.getByRole("alert")).toContainText("the disk is full");
});

test("a shortcut can be recorded", async ({ page }) => {
  await page.getByRole("button", { name: "Shortcuts", exact: true }).click();
  const row = page.locator(".row", { hasText: /^Mute/ });
  await row.getByRole("button").first().click();
  await expect(row.getByRole("button").first()).toHaveText("Press keys…");
  await page.keyboard.press("y");
  await expect.poll(async () => (await calls(page, "settings_set")).at(-1)).toMatchObject({ settings: { shortcuts: { mute: "Y" } } });
});

test("shortcuts are written the way this platform writes them", async ({ page }, info) => {
  await page.getByRole("button", { name: "Shortcuts", exact: true }).click();
  const text = await page.locator("main").innerText();
  if (isMacProject(info.project.name)) expect(text).toContain("⌘");
  else {
    expect(text).toContain("Ctrl+");
    expect(text).not.toContain("⌘");
  }
});
