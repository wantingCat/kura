import { api, events, type MpvStatus, type InstallMpvResult } from "./api";
import { app } from "./app.svelte";

class MpvModalState {
  visible = $state(false);
  status = $state<MpvStatus | null>(null);
  installing = $state(false);
  progressMessage = $state<string | null>(null);
  result = $state<InstallMpvResult | null>(null);
  error = $state<string | null>(null);

  open() {
    this.visible = true;
    this.error = null;
    this.result = null;
    this.check();
  }

  close() {
    this.visible = false;
    this.installing = false;
    this.progressMessage = null;
  }

  async dismissPermanently() {
    try {
      await api.setPref("mpv_prompt_dismissed", "1");
    } catch {}
    if (typeof localStorage !== "undefined") {
      try {
        localStorage.setItem("kura_mpv_prompt_dismissed", "true");
      } catch {}
    }
    this.close();
  }

  async check(): Promise<MpvStatus | null> {
    try {
      this.status = await api.checkMpvStatus();
      return this.status;
    } catch {
      this.status = null;
      return null;
    }
  }

  async checkInitialLaunch() {
    // Check local storage fast-path
    if (typeof localStorage !== "undefined" && localStorage.getItem("kura_mpv_prompt_dismissed") === "true") {
      return;
    }
    // Check persistent database preferences
    try {
      const prefs = await api.getPrefs();
      if (prefs.mpv_prompt_dismissed === "1") {
        return;
      }
    } catch {}

    try {
      const s = await api.checkMpvStatus();
      this.status = s;
      if (!s.installed) {
        this.visible = true;
      }
    } catch {
      /* ignore */
    }
  }

  async install() {
    if (this.installing) return;
    this.installing = true;
    this.error = null;
    this.result = null;
    this.progressMessage = this.status?.hasWinget
      ? "Installing MPV via Windows Package Manager (winget)…"
      : this.status?.hasChoco
      ? "Installing MPV via Chocolatey…"
      : "Preparing installation…";

    let unlisten: (() => void) | null = null;
    try {
      unlisten = await events.onMpvInstallProgress((msg) => {
        this.progressMessage = msg;
      });
    } catch {
      /* ignore listener error */
    }

    try {
      const res = await api.installMpv();
      this.result = res;
      if (res.success) {
        await app.setPlayback("player", "mpv");
        if (res.path) {
          await app.setPlayback("playerPath", res.path);
        }
        app.toast("MPV successfully configured!", "info");
        await this.check();
      } else if (res.method === "website") {
        this.progressMessage = "Opened MPV download page in your browser.";
      } else {
        this.error = res.message;
      }
    } catch (e) {
      this.error = String(e);
    } finally {
      this.installing = false;
      if (unlisten) unlisten();
    }
  }
}

export const mpvModal = new MpvModalState();
