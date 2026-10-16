import { mount } from "svelte";
import App from "./App.svelte";
import SettingsApp from "./lib/SettingsApp.svelte";
import "./app.css";
import { isMac } from "./lib/platform";
import { installTooltips } from "./lib/tooltip";

// Lets styles make room for the macOS traffic lights or the Windows window buttons.
document.documentElement.dataset.platform = isMac ? "mac" : "windows";

// The webview's own right-click menu (Back, Reload, Print…) isn't the app's.
// Text fields keep theirs for cut, copy and paste.
document.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement | null;
  if (!t?.closest("input, textarea, [contenteditable]")) e.preventDefault();
});

installTooltips();

// One bundle serves both windows; the settings window opens with ?view=settings.
const view = new URLSearchParams(location.search).get("view");
export default mount(view === "settings" ? SettingsApp : App, { target: document.getElementById("app")! });
