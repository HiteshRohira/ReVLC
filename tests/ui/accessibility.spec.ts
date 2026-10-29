import AxeBuilder from "@axe-core/playwright";
import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import { SCREENS } from "./screens";

// axe-core's WCAG 2.1 A and AA rules on each main screen. Colour contrast
// is checked on its own below, across every screen at once.
const axe = (page: Page) => new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"]);
const summary = (violations: { id: string; nodes: { target: unknown[] }[] }[]) =>
  violations.map((v) => `${v.id}: ${v.nodes.slice(0, 3).map((n) => n.target.join(" ")).join(", ")}`);

// Open problems, by screen: checked on their own as known bugs, so the
// rest of the screen is still checked.
const KNOWN: Record<string, { rule: string; bug: string }> = {};

for (const [name, show] of Object.entries(SCREENS)) {
  test(`${name}: no accessibility problems`, async ({ page }) => {
    await show(page);
    const known = KNOWN[name] ? [KNOWN[name].rule] : [];
    const { violations } = await axe(page).disableRules(["color-contrast", ...known]).analyze();
    expect(summary(violations)).toEqual([]);
  });
}

for (const [name, { rule, bug }] of Object.entries(KNOWN)) {
  test(`${name}: ${rule}`, async ({ page }) => {
    test.fail(true, bug);
    await SCREENS[name](page);
    const { violations } = await axe(page).withRules([rule]).analyze();
    expect(summary(violations)).toEqual([]);
  });
}

test("text has enough contrast everywhere", async ({ page }) => {
  const found: string[] = [];
  for (const [name, show] of Object.entries(SCREENS)) {
    await show(page);
    const { violations } = await axe(page).withRules(["color-contrast"]).analyze();
    found.push(...summary(violations).map((v) => `${name}: ${v}`));
  }
  expect(found).toEqual([]);
});
