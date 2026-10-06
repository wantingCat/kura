import {
  api,
  events,
  type Library,
  type MediaCard,
  type PlaybackEvent,
  type PlayerKind,
  type ScanProgress,
  type UnmatchedGroup,
} from "./api";
import { updater } from "./updater.svelte";

export interface PlaybackPrefs {
  /** Empty = not chosen yet (the backend then uses the first detected player). */
  player: PlayerKind | "";
  playerPath: string;
  /** Percent of the episode after which it counts as watched (50–100). */
  threshold: number;
  resume: boolean;
  autoplay: boolean;
  mpcPort: number;
}

export interface Toast {
  id: number;
  kind: "info" | "warn" | "error";
  text: string;
}

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
  /** Library search, driven by the top bar. */
  query = $state("");
  playback = $state<PlaybackPrefs>({
    player: "",
    playerPath: "",
    threshold: 90,
    resume: true,
    autoplay: false,
    mpcPort: 13579,
  });
  autoUpdate = $state(true);
  /** Latest playback event while a player is being tracked. */
  nowPlaying = $state<PlaybackEvent | null>(null);
  toasts = $state<Toast[]>([]);

  #refreshTimer: ReturnType<typeof setTimeout> | null = null;
  #initialised = false;
  #toastId = 0;

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
    await events.onPlayback((p) => {
      if (p.state === "tracking") this.nowPlaying = p;
      else if (p.state === "stopped") this.nowPlaying = null;
      else {
        this.nowPlaying = null;
        if (p.hint) this.toast(p.hint, "warn", 9000);
      }
    });
    this.scanning = await api.isScanning();
    try {
      const prefs = await api.getPrefs();
      this.groupSeasons = prefs.group_seasons !== "0";
      this.autoUpdate = prefs.auto_update !== "0";
      const num = (v: string | undefined, d: number) => (v !== undefined && !isNaN(Number(v)) ? Number(v) : d);
      this.playback = {
        player: (prefs.player as PlayerKind) ?? "",
        playerPath: prefs.player_path ?? "",
        threshold: Math.min(100, Math.max(50, num(prefs.watched_threshold, 90))),
        resume: prefs.resume !== "0",
        autoplay: prefs.autoplay === "1",
        mpcPort: num(prefs.mpc_port, 13579),
      };
    } catch {
      /* keep defaults */
    }
    await this.refresh();
    if (this.autoUpdate) setTimeout(() => updater.check(), 3000);
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

  async setAutoUpdate(on: boolean) {
    this.autoUpdate = on;
    await api.setPref("auto_update", on ? "1" : "0");
  }

  async setPlayback<K extends keyof PlaybackPrefs>(key: K, value: PlaybackPrefs[K]) {
    this.playback[key] = value;
    const names: Record<keyof PlaybackPrefs, string> = {
      player: "player",
      playerPath: "player_path",
      threshold: "watched_threshold",
      resume: "resume",
      autoplay: "autoplay",
      mpcPort: "mpc_port",
    };
    const v = typeof value === "boolean" ? (value ? "1" : "0") : String(value);
    await api.setPref(names[key], v);
  }

  /** Play an episode with the configured player; errors become a toast. */
  async play(anilistId: number, epKey: string, path: string) {
    try {
      await api.playEpisode(anilistId, epKey, path);
    } catch (e) {
      this.toast(String(e), "error", 8000);
    }
  }

  toast(text: string, kind: Toast["kind"] = "info", ms = 5000) {
    const id = ++this.#toastId;
    this.toasts = [...this.toasts, { id, kind, text }];
    setTimeout(() => this.dismissToast(id), ms);
  }

  dismissToast(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  get reviewCount() {
    return this.unmatched.length + this.cards.filter((c) => c.needsReview).length;
  }
}

export const app = new AppStore();
