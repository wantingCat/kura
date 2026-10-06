import { isTauri } from "@tauri-apps/api/core";
import type { Update } from "@tauri-apps/plugin-updater";

type Status = "idle" | "checking" | "none" | "available" | "downloading" | "installing" | "error";

/**
 * Self-update via the Tauri updater plugin. Releases are signed in CI and
 * published to GitHub; `latest.json` on the newest release tells us what's new.
 */
class Updater {
  status = $state<Status>("idle");
  version = $state<string | null>(null);
  notes = $state<string | null>(null);
  downloaded = $state(0);
  total = $state(0);
  error = $state<string | null>(null);
  dismissed = $state(false);
  #update: Update | null = null;

  get visible() {
    return !this.dismissed && ["available", "downloading", "installing", "error"].includes(this.status) && !!this.version;
  }

  get percent() {
    return this.total > 0 ? Math.min(100, (this.downloaded / this.total) * 100) : null;
  }

  async check({ silent = true } = {}) {
    // Browser preview: `?preview=update` fakes an available update for UI work.
    if (!isTauri()) {
      if (new URLSearchParams(location.search).get("preview") === "update") {
        this.version = "0.2.1";
        this.status = "available";
      } else {
        this.status = "none";
      }
      return;
    }
    if (this.status === "downloading" || this.status === "installing") return;
    this.status = "checking";
    this.error = null;
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const u = await check();
      this.#update = u;
      if (u) {
        this.version = u.version;
        this.notes = u.body ?? null;
        this.status = "available";
        this.dismissed = false;
      } else {
        this.status = "none";
      }
    } catch (e) {
      this.status = silent ? "idle" : "error";
      this.error = String(e);
      console.warn("[kura] update check failed", e);
    }
  }

  async install() {
    const u = this.#update;
    if (!u) return;
    this.status = "downloading";
    this.downloaded = 0;
    this.total = 0;
    try {
      await u.downloadAndInstall((ev) => {
        if (ev.event === "Started") this.total = ev.data.contentLength ?? 0;
        else if (ev.event === "Progress") this.downloaded += ev.data.chunkLength;
        else if (ev.event === "Finished") this.status = "installing";
      });
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch (e) {
      this.status = "error";
      this.error = String(e);
    }
  }
}

export const updater = new Updater();
