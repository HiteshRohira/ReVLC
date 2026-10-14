import { mount } from "svelte";
import App from "./App.svelte";
import SettingsApp from "./lib/SettingsApp.svelte";
import "./app.css";
import { isMac } from "./lib/platform";

// Lets styles make room for the macOS traffic lights or the Windows window buttons.
document.documentElement.dataset.platform = isMac ? "mac" : "windows";

// One bundle serves both windows; the settings window opens with ?view=settings.
const view = new URLSearchParams(location.search).get("view");
export default mount(view === "settings" ? SettingsApp : App, { target: document.getElementById("app")! });
