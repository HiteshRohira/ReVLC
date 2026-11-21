import test from "node:test";
import assert from "node:assert/strict";
import { detectDesktopOS } from "./download-platform.js";

const cases = [
  ["Mac Chromium", { userAgentData: { platform: "macOS", mobile: false }, platform: "MacIntel", userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)" }, "mac"],
  ["Mac Safari without client hints", { platform: "MacIntel", userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)" }, "mac"],
  ["Windows Chromium", { userAgentData: { platform: "Windows" }, platform: "Win32", userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)" }, "windows"],
  ["Windows Firefox", { platform: "Win32", userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:144.0)" }, "windows"],
  ["Linux Chromium", { userAgentData: { platform: "Linux" }, platform: "Linux x86_64", userAgent: "Mozilla/5.0 (X11; Linux x86_64)" }, "linux"],
  ["Linux Firefox", { platform: "Linux x86_64", userAgent: "Mozilla/5.0 (X11; Ubuntu; Linux x86_64)" }, "linux"],
  ["client hints alone", { userAgentData: { platform: "Windows" } }, "windows"],
  ["UA alone", { userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)" }, "mac"],
  ["iPad desktop mode", { platform: "MacIntel", userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", maxTouchPoints: 5 }, null],
  ["iPad with hints", { userAgentData: { platform: "macOS", mobile: false }, platform: "MacIntel", maxTouchPoints: 5 }, null],
  ["iPhone", { platform: "iPhone", userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)" }, null],
  ["iPad mobile UA", { platform: "iPad", userAgent: "Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X)" }, null],
  ["Android with Linux platform", { userAgentData: { platform: "Android", mobile: true }, platform: "Linux armv8l", userAgent: "Mozilla/5.0 (Linux; Android 15)" }, null],
  ["Android tablet desktop mode", { platform: "Linux x86_64", userAgent: "Mozilla/5.0 (X11; Linux x86_64) Android 15" }, null],
  ["ChromeOS", { userAgentData: { platform: "Chrome OS" }, platform: "Linux x86_64", userAgent: "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0)" }, null],
  ["touchscreen Windows is not iPad", { platform: "Win32", userAgent: "Windows NT 10.0", maxTouchPoints: 10 }, "windows"],
  ["conflicting hints and platform", { userAgentData: { platform: "macOS" }, platform: "Win32" }, null],
  ["conflicting platform and UA", { platform: "Linux x86_64", userAgent: "Windows NT 10.0" }, null],
  ["Linux ARM", { platform: "Linux aarch64", userAgent: "Mozilla/5.0 (X11; Linux aarch64)" }, null],
  ["Linux ARM client hints", { userAgentData: { platform: "Linux", architecture: "arm" } }, null],
  ["32-bit Linux", { platform: "Linux i686", userAgent: "Mozilla/5.0 (X11; Linux i686)" }, null],
  ["Windows CE is not supported desktop Windows", { platform: "wince", userAgent: "Windows CE" }, null],
  ["unknown desktop", { platform: "FreeBSD amd64", userAgent: "Mozilla/5.0 (X11; FreeBSD amd64)" }, null],
  ["empty navigator", {}, null],
  ["missing navigator", undefined, null],
];
for (const [name, input, expected] of cases) test(name, () => assert.equal(detectDesktopOS(input), expected));
