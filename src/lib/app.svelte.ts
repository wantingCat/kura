import { api, events, type Library, type MediaCard, type ScanProgress, type UnmatchedGroup } from "./api";

class AppStore {
  libraries = $state<Library[]>([]);
  cards = $state<MediaCard[]>([]);
  unmatched = $state<UnmatchedGroup[]>([]);
  loaded = $state(false);
  scanning = $state(false);
  progress = $state<ScanProgress | null>(null);
  lastError = $state<string | null>(null);
  /** Bumped on every library change so pages can re-fetch their own data. */
  version = $state(0);
  /** Show one card per franchise (seasons as tabs) instead of one per AniList entry. */
  groupSeasons = $state(true);

  #refreshTimer: ReturnType<typeof setTimeout> | null = null;
  #initialised = false;

  async init() {
    if (this.#initialised) return;
    this.#initialised = true;
    await events.onLibraryChanged(() => this.scheduleRefresh());
    await events.onScanProgress((p) => {
      this.scanning = p.phase !== "done";
      this.progress = p;
    });
    await events.onScanFinished(() => {
      this.scanning = false;
      setTimeout(() => {
        if (!this.scanning) this.progress = null;
      }, 2500);
      this.scheduleRefresh();
    });
    await events.onScanError((msg) => (this.lastError = msg));
    this.scanning = await api.isScanning();
    try {
      const prefs = await api.getPrefs();
      this.groupSeasons = prefs.group_seasons !== "0";
    } catch {
      /* keep defaults */
    }
    await this.refresh();
  }

  scheduleRefresh() {
    if (this.#refreshTimer) clearTimeout(this.#refreshTimer);
    this.#refreshTimer = setTimeout(() => this.refresh(), 250);
  }

  async refresh() {
    const [libraries, cards, unmatched] = await Promise.all([
      api.listLibraries(),
      api.getLibrary(null),
      api.getUnmatched(),
    ]);
    this.libraries = libraries;
    this.cards = cards;
    this.unmatched = unmatched;
    this.loaded = true;
    this.version++;
  }

  async scan(libraryId: number | null = null) {
    this.scanning = true;
    this.lastError = null;
    this.progress = { phase: "discover", current: 0, total: 0, message: "Starting scan…" };
    await api.startScan(libraryId);
  }

  async setGroupSeasons(on: boolean) {
    this.groupSeasons = on;
    await api.setPref("group_seasons", on ? "1" : "0");
  }

  get reviewCount() {
    return this.unmatched.length + this.cards.filter((c) => c.needsReview).length;
  }
}

export const app = new AppStore();
