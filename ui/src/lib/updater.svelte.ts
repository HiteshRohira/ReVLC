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

  /** Whether to show the update banner. Background downloads stay quiet. */
  prompt = $derived.by(() => {
    const { status, byUser, version } = this.state;
    const shown = status === "available" || status === "ready" || (status === "downloading" && byUser);
    return shown && this.dismissed !== version;
  });

  async start() {
    await listen<UpdateState>("update://state", (e) => (this.state = e.payload));
    // A check the user asked for shows the banner again after "Later".
    await listen("update://manual-check", () => (this.dismissed = ""));
    this.state = await invoke<UpdateState>("update_state");
  }

  install = () => invoke("update_install");
  restart = () => invoke("update_restart");
  later = () => (this.dismissed = this.state.version);
}

export const updater = new Updater();
