import { api } from "./api";
import { getVersion } from "@tauri-apps/api/app";
import { CHANGELOG, type ChangelogRelease } from "./changelog";

class ChangelogModalState {
  visible = $state(false);
  currentRelease = $state<ChangelogRelease>(CHANGELOG[0]);
  activeVersion = $state<string>(CHANGELOG[0].version);
  #appVersion: string | null = null;

  async getAppVersion(): Promise<string> {
    if (this.#appVersion) return this.#appVersion;
    try {
      this.#appVersion = await getVersion();
      return this.#appVersion;
    } catch {
      return CHANGELOG[0].version;
    }
  }

  open(version?: string) {
    if (version) {
      const found = CHANGELOG.find((r) => r.version === version);
      if (found) {
        this.currentRelease = found;
        this.activeVersion = found.version;
      }
    } else {
      this.currentRelease = CHANGELOG[0];
      this.activeVersion = CHANGELOG[0].version;
    }
    this.visible = true;
  }

  selectVersion(v: string) {
    const found = CHANGELOG.find((r) => r.version === v);
    if (found) {
      this.currentRelease = found;
      this.activeVersion = v;
    }
  }

  async close() {
    this.visible = false;
    try {
      const v = await this.getAppVersion();
      await api.setPref("last_seen_version", v);
    } catch (e) {
      console.warn("[kura] failed to save last_seen_version", e);
    }
  }

  async checkOnLaunch() {
    try {
      const currentVersion = await this.getAppVersion();
      const prefs = await api.getPrefs();
      const lastSeen = prefs.last_seen_version;

      // If this version hasn't been seen yet, show the changelog popup!
      if (!lastSeen || lastSeen !== currentVersion) {
        this.open(currentVersion);
      }
    } catch (e) {
      console.warn("[kura] changelog launch check failed", e);
    }
  }
}

export const changelogModal = new ChangelogModalState();
