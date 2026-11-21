// Browser platform strings can be reduced or overridden. Conflicting signals
// and mobile devices get the chooser rather than a guessed desktop download.
export function detectDesktopOS({ userAgentData, platform = "", userAgent = "", maxTouchPoints = 0 } = {}) {
  const hint = userAgentData?.platform || "";
  const all = `${hint} ${platform} ${userAgent}`;
  if (userAgentData?.mobile || /Android|iPhone|iPad|iPod|Windows Phone|CrOS|Chrome OS|Chromium OS|\bMobi\b/i.test(all)) return null;
  // iPadOS can identify itself as a Mac, including in desktop browsing mode.
  if (maxTouchPoints > 1 && /Mac/i.test(all)) return null;

  const fromPlatform = (value) => {
    if (/^(macOS|MacIntel|MacPPC|Mac68K)$/i.test(value)) return "mac";
    if (/^(Windows|Win32|Win64)$/i.test(value)) return "windows";
    if (/^Linux(?:\s|$)/i.test(value)) return "linux";
    return null;
  };
  const fromUA = /Windows NT/i.test(userAgent) ? "windows"
    : /Macintosh|Mac OS X/i.test(userAgent) ? "mac"
    : /\bLinux\b/i.test(userAgent) ? "linux" : null;
  const signals = new Set([fromPlatform(hint), fromPlatform(platform), fromUA].filter(Boolean));
  if (signals.size !== 1) return null;
  const os = [...signals][0];
  // There is only an amd64 Linux package. Mac UA strings saying "Intel" do
  // NOT establish CPU architecture on modern Macs, so always label that
  // download explicitly as Apple Silicon instead of claiming compatibility.
  if (os === "linux" && (/arm|aarch64|\bi[3-6]86\b/i.test(`${platform} ${userAgent} ${userAgentData?.architecture || ""}`) || userAgentData?.bitness === "32")) return null;
  return os;
}
