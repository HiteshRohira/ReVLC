import { test, expect } from "./fixtures";
import { SCREENS } from "./screens";

// Each main screen against its saved picture, to catch layout and style
// changes no other test looks at. Pictures depend on the fonts, so they're
// only taken in the Playwright Linux container CI uses (the "screenshots"
// job in ui-checks.yml); other machines skip these.
//
// After an intended change, run that job with "update screenshots" (Actions
// › UI checks › Run workflow) and commit the pictures from its artifact.
test.skip(!process.env.REVLC_SCREENSHOTS, "only in the screenshots CI job (pinned fonts)");

for (const [name, show] of Object.entries(SCREENS)) {
  test(`${name} looks as saved @screens`, async ({ page }) => {
    await show(page);
    await expect(page).toHaveScreenshot(`${name.replace(/ /g, "-")}.png`, { animations: "disabled", caret: "hide", maxDiffPixelRatio: 0.002 });
  });
}
