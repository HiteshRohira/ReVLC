import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Status = "idle" | "checking" | "available" | "downloading" | "ready" | "error";

/** Mirrors `UpdateState` in src-tauri/src/updater.rs, which does the work. */
interface UpdateState {
  status: Status;
  version: string;
  progress: number | null;
  byUser: boolean;
}

class Updater {
  state = $state<UpdateState>({ status: "idle", version: "", progress: null, byUser: false });
  /** The version the user said "Later" to; not offered again this session. */
  dismissed = $state("");

  /** Whether to show the restart banner; the sidebar button shows the rest. */
  prompt = $derived(this.state.status === "ready" && this.dismissed !== this.state.version);

  /** What the sidebar button says, and does on click. */
  tip = $derived.by(() => {
    const { status, version, progress } = this.state;
    switch (status) {
      case "checking":
        return "Checking for updates…";
      case "available":
        return `Update to ReVLC ${version}`;
      case "downloading":
        return `Downloading ReVLC ${version}${progress === null ? "…" : ` (${Math.floor(progress * 100)}%)`}`;
      case "ready":
        return `Restart to update to ReVLC ${version}`;
      case "error":
        return "Couldn't check for updates. Click to try again.";
      default:
        return "Check for updates";
    }
  });

  async start() {
    await listen<UpdateState>("update://state", (e) => (this.state = e.payload));
    // A check the user asked for shows the banner again after "Later".
    await listen("update://manual-check", () => (this.dismissed = ""));
    this.state = await invoke<UpdateState>("update_state");
  }

  act = () => {
    const { status } = this.state;
    if (status === "idle" || status === "error") invoke("update_check");
    else if (status === "available") invoke("update_install");
    else if (status === "ready") invoke("update_restart");
  };

  restart = () => invoke("update_restart");
  later = () => (this.dismissed = this.state.version);
}

export const updater = new Updater();
