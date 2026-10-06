import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface Folder {
  id: number;
  path: string;
  exists: boolean;
}

export interface Library {
  id: number;
  name: string;
  kind: string;
  folders: Folder[];
  fileCount: number;
  mediaCount: number;
}

export interface MediaCard {
  anilistId: number;
  titleRomaji: string | null;
  titleEnglish: string | null;
  titleNative: string | null;
  synonyms: string[];
  format: string | null;
  status: string | null;
  episodes: number | null;
  season: string | null;
  seasonYear: number | null;
  genres: string[];
  averageScore: number | null;
  coverUrl: string | null;
  coverPath: string | null;
  coverColor: string | null;
  bannerUrl: string | null;
  bannerPath: string | null;
  description: string | null;
  ownedCount: number;
  watchedCount: number;
  addedAt: number;
  lastWatchedAt: number | null;
  needsReview: boolean;
  libraryIds: number[];
  /** AniList id of the first owned entry of this franchise (own id if standalone). */
  franchiseId: number;
  /** Position among owned entries of the franchise, in release order. */
  franchiseIndex: number;
  franchiseSize: number;
}

export interface UnmatchedGroup {
  id: number;
  libraryId: number;
  displayName: string;
  folderPath: string | null;
  titleGuess: string;
  fileCount: number;
  confidence: number | null;
  attempted: boolean;
  sampleFiles: string[];
}

export interface FileRef {
  id: number;
  path: string;
  fileName: string;
  size: number;
}

export interface EpisodeRow {
  epKey: string;
  number: number;
  isSpecial: boolean;
  titleEn: string | null;
  titleJa: string | null;
  titleRomaji: string | null;
  overview: string | null;
  airDate: string | null;
  runtime: number | null;
  thumbUrl: string | null;
  thumbPath: string | null;
  filler: boolean;
  recap: boolean;
  files: FileRef[];
  watchedAt: number | null;
  /** Saved resume point in seconds. */
  progressPos: number | null;
  progressDur: number | null;
}

export interface RelationCard {
  relatedId: number;
  relationType: string;
  title: string | null;
  format: string | null;
  status: string | null;
  episodes: number | null;
  seasonYear: number | null;
  coverUrl: string | null;
  coverPath: string | null;
  ownedCount: number;
}

export interface GroupRef {
  id: number;
  displayName: string;
  folderPath: string | null;
  confidence: number | null;
  manual: boolean;
}

export interface FranchiseEntry {
  anilistId: number;
  titleRomaji: string | null;
  titleEnglish: string | null;
  titleNative: string | null;
  format: string | null;
  status: string | null;
  season: string | null;
  seasonYear: number | null;
  episodes: number | null;
  ownedCount: number;
  watchedCount: number;
}

export interface MediaDetail {
  anilistId: number;
  idMal: number | null;
  titleRomaji: string | null;
  titleEnglish: string | null;
  titleNative: string | null;
  synonyms: string[];
  format: string | null;
  status: string | null;
  episodes: number | null;
  duration: number | null;
  season: string | null;
  seasonYear: number | null;
  startDate: string | null;
  endDate: string | null;
  description: string | null;
  genres: string[];
  tags: string[];
  studios: string[];
  coverUrl: string | null;
  coverPath: string | null;
  coverColor: string | null;
  bannerUrl: string | null;
  bannerPath: string | null;
  averageScore: number | null;
  nextAiringEpisode: number | null;
  nextAiringAt: number | null;
  episodesSource: string | null;
  episodeList: EpisodeRow[];
  relations: RelationCard[];
  otherFiles: FileRef[];
  groups: GroupRef[];
  /** Owned entries of the same franchise in release order (empty if standalone). */
  franchise: FranchiseEntry[];
}

export interface SearchResult {
  anilistId: number;
  titleRomaji: string | null;
  titleEnglish: string | null;
  titleNative: string | null;
  format: string | null;
  status: string | null;
  episodes: number | null;
  seasonYear: number | null;
  coverUrl: string | null;
}

export interface ScanProgress {
  phase: "discover" | "match" | "resolve" | "metadata" | "artwork" | "done";
  current: number;
  total: number;
  message: string;
}

/** An episode surfaced on the home page (continue watching / new episode). */
export interface UpNextItem {
  anilistId: number;
  epKey: string;
  number: number;
  path: string;
  titleRomaji: string | null;
  titleEnglish: string | null;
  titleNative: string | null;
  format: string | null;
  episodes: number | null;
  coverUrl: string | null;
  coverPath: string | null;
  coverColor: string | null;
  bannerUrl: string | null;
  bannerPath: string | null;
  episodeTitle: string | null;
  thumbUrl: string | null;
  thumbPath: string | null;
  runtime: number | null;
  /** Seconds; 0 when not started. */
  position: number;
  duration: number;
  ownedCount: number;
  watchedCount: number;
  at: number;
  newCount: number;
}

export type PlayerKind = "system" | "mpv" | "vlc" | "mpc" | "memento";

export interface DetectedPlayer {
  kind: PlayerKind;
  path: string;
}

export interface PlaybackEvent {
  state: "tracking" | "untracked" | "stopped";
  player: PlayerKind;
  anilistId: number;
  epKey: string;
  position: number;
  duration: number;
  hint: string | null;
}

export const api = {
  listLibraries: () => invoke<Library[]>("list_libraries"),
  createLibrary: (name: string, folders: string[]) => invoke<number>("create_library", { name, folders }),
  renameLibrary: (id: number, name: string) => invoke<void>("rename_library", { id, name }),
  deleteLibrary: (id: number) => invoke<void>("delete_library", { id }),
  addFolder: (libraryId: number, path: string) => invoke<void>("add_folder", { libraryId, path }),
  removeFolder: (folderId: number) => invoke<void>("remove_folder", { folderId }),
  startScan: (libraryId: number | null = null) => invoke<void>("start_scan", { libraryId }),
  isScanning: () => invoke<boolean>("is_scanning"),
  getLibrary: (libraryId: number | null = null) => invoke<MediaCard[]>("get_library", { libraryId }),
  getUnmatched: () => invoke<UnmatchedGroup[]>("get_unmatched"),
  getMediaDetail: (anilistId: number) => invoke<MediaDetail>("get_media_detail", { anilistId }),
  setWatched: (anilistId: number, epKeys: string[], watched: boolean) =>
    invoke<void>("set_watched", { anilistId, epKeys, watched }),
  searchAnilist: (query: string) => invoke<SearchResult[]>("search_anilist", { query }),
  fixMatch: (newAnilistId: number, opts: { groupId?: number; fromAnilistId?: number }) =>
    invoke<void>("fix_match", {
      newAnilistId,
      groupId: opts.groupId ?? null,
      fromAnilistId: opts.fromAnilistId ?? null,
    }),
  refreshMedia: (anilistId: number) => invoke<void>("refresh_media", { anilistId }),
  openFile: (path: string) => invoke<void>("open_file", { path }),
  revealFile: (path: string) => invoke<void>("reveal_file", { path }),
  getPrefs: () => invoke<Record<string, string>>("get_prefs"),
  setPref: (key: string, value: string) => invoke<void>("set_pref", { key, value }),
  detectPlayers: () => invoke<DetectedPlayer[]>("detect_players"),
  playEpisode: (anilistId: number, epKey: string, path: string) =>
    invoke<void>("play_episode", { anilistId, epKey, path }),
  getUpNext: (limit = 6) => invoke<UpNextItem[]>("get_up_next", { limit }),
  getNewEpisodes: (limit = 6) => invoke<UpNextItem[]>("get_new_episodes", { limit }),
};

export const events = {
  onLibraryChanged: (cb: () => void): Promise<UnlistenFn> => listen("library-changed", () => cb()),
  onScanProgress: (cb: (p: ScanProgress) => void): Promise<UnlistenFn> =>
    listen<ScanProgress>("scan-progress", (e) => cb(e.payload)),
  onScanFinished: (cb: () => void): Promise<UnlistenFn> => listen("scan-finished", () => cb()),
  onScanError: (cb: (msg: string) => void): Promise<UnlistenFn> => listen<string>("scan-error", (e) => cb(e.payload)),
  onPlayback: (cb: (p: PlaybackEvent) => void): Promise<UnlistenFn> =>
    listen<PlaybackEvent>("playback", (e) => cb(e.payload)),
};

/** Prefer the locally cached image (works offline), fall back to the remote URL. */
export function img(localPath: string | null | undefined, remote: string | null | undefined): string | null {
  if (localPath) return convertFileSrc(localPath);
  return remote ?? null;
}
