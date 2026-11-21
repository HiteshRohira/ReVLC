import { expect, type Page } from "@playwright/test";
import { open } from "./fixtures";

/** The screens the accessibility and screenshot tests look at. */
export const SCREENS: Record<string, (page: Page) => Promise<void>> = {
  welcome: (page) => open(page, "home", { onboarded: false }),
  home: async (page) => {
    await open(page, "home");
    await expect(page.getByRole("heading", { name: "Continue watching" })).toBeVisible();
  },
  player: async (page) => {
    await open(page, "playing");
    await expect(page.getByRole("button", { name: "Pause" })).toBeVisible();
  },
  "media information": async (page) => {
    await SCREENS.player(page);
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: /^Media information/ }).click();
    await expect(page.getByRole("dialog", { name: "Media information" })).toBeVisible();
  },
  "shortcut list": async (page) => {
    await SCREENS.player(page);
    await page.keyboard.press("?");
    await expect(page.getByRole("dialog", { name: "Keyboard shortcuts" })).toBeVisible();
  },
  settings: async (page) => {
    await open(page, "home", { settings: true });
    await expect(page.getByRole("button", { name: "General" }).first()).toBeVisible();
  },
};
