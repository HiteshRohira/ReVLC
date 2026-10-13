import { mount } from "svelte";
import App from "./App.svelte";
import SettingsApp from "./lib/SettingsApp.svelte";
import "./app.css";

// One bundle serves both windows; the settings window opens with ?view=settings.
const view = new URLSearchParams(location.search).get("view");
export default mount(view === "settings" ? SettingsApp : App, { target: document.getElementById("app")! });
