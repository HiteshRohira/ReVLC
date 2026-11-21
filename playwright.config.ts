import { defineConfig } from "@playwright/test";

// Browser tests of ReVLC's UI with the Rust backend mocked (ui/tests/mock.ts).
// The UI picks its macOS or Windows/Linux layout from the user agent, so
// each test runs as both: macOS in WebKit (the engine of the macOS app's
// web view), Windows in Chromium (WebView2's engine).
const MAC_UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";
const WINDOWS_UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0";

export default defineConfig({
  testDir: "tests/ui",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: { baseURL: "http://localhost:1431", viewport: { width: 1280, height: 800 }, trace: "retain-on-failure" },
  projects: [
    { name: "macos", use: { browserName: "webkit", userAgent: MAC_UA } },
    { name: "windows", use: { browserName: "chromium", userAgent: WINDOWS_UA } },
  ],
  webServer: {
    // Its own port, so it never reuses a dev server from another checkout.
    command: "npx vite --port 1431 --strictPort",
    url: "http://localhost:1431/tests/harness.html",
    reuseExistingServer: false,
  },
});
