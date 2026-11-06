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
  /** Non-spoiler AniList tags. */
  tags: string[];
  /** Main animation studios. */
  studios: string[];
  averageScore: number | null;
  coverUrl: string | null;
  coverPath: string | null;
  coverColor: string | null;
  bannerUrl: string | null;
  bannerPath: string | null;
  logoUrl: string | null;
  logoPath: string | null;
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
  anilistStatus: string | null;
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

export interface ExtraFile {
  id: number;
  path: string;
  fileName: string;
  title: string;
  kind: "opening" | "ending" | "trailer" | "pv" | "bonus" | "other";
  size: number;
}

export interface MediaTrackPref {
  audioPref: string | null;
  subPref: string | null;
  subFallback: string | null;
}

export interface LinkedSubtitle {
  id: number;
  anilistId: number;
  epKey: string;
  path: string;
  fileName: string;
  lang: string;
  cc: boolean;
  source: string;
  retimed: boolean;
  offsetMs: number | null;
  exists: boolean;
}

export interface CharacterCard {
  characterId: number;
  role: string;
  nameFull: string;
  nameNative: string | null;
  imageUrl: string | null;
  vaId: number | null;
  vaNameFull: string | null;
  vaNameNative: string | null;
  vaImageUrl: string | null;
}

export interface StaffCard {
  staffId: number;
  role: string;
  nameFull: string;
  nameNative: string | null;
  imageUrl: string | null;
}

export interface PersonWorkItem {
  anilistId: number;
  titleRomaji: string | null;
  titleEnglish: string | null;
  titleNative: string | null;
  format: string | null;
  seasonYear: number | null;
  coverUrl: string | null;
  coverPath: string | null;
  role: string;
  characterName: string | null;
  characterImage: string | null;
  inLibrary: boolean;
  ownedCount: number;
}

export interface PersonDetail {
  staffId: number;
  nameFull: string;
  nameNative: string | null;
  imageUrl: string | null;
  occupations: string[];
  works: PersonWorkItem[];
}

export interface SubtitleMatchItem {
  sourcePath: string;
  sourceName: string;
  epKey: string;
  episodeNumber: number;
  videoPath: string;
  videoName: string;
  targetPath: string;
  targetName: string;
  lang: string;
  cc: boolean;
  resolutionNote: string | null;
  alreadyLinked: boolean;
}

export interface SubtitleSkipItem {
  sourcePath: string;
  sourceName: string;
  reason: string;
}

export interface SubtitleMatchPreview {
  anilistId: number;
  scannedFolders: string[];
  matches: SubtitleMatchItem[];
  skipped: SubtitleSkipItem[];
}

export interface SubtitleApplyReport {
  linked: number;
  renamed: number;
  retimed: number;
  errors: string[];
}

export interface RetimeReport {
  subtitlePath: string;
  outputPath: string;
  offsetMs: number;
  segments: number;
  matchedCues: number;
  totalCues: number;
  referenceSource: string;
  loadedIntoMpv: boolean;
}

export interface JimakuEntry {
  id: number;
  name: string;
  englishName: string | null;
  japaneseName: string | null;
  anilistId: number | null;
}

export interface JimakuFileItem {
  name: string;
  url: string;
  size: number;
  lastModified: string | null;
  isArchive: boolean;
  isRar: boolean;
  detectedEpisode: number | null;
  matchScore: number;
}

export interface JimakuSearchResponse {
  configured: boolean;
  entry: JimakuEntry | null;
  files: JimakuFileItem[];
}

export interface JimakuDownloadResult {
  savedPaths: string[];
  linkedCount: number;
  retimeReport: RetimeReport | null;
  loadedIntoMpv: boolean;
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
  subtitles?: LinkedSubtitle[];
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
  logoUrl: string | null;
  logoPath: string | null;
  averageScore: number | null;
  nextAiringEpisode: number | null;
  nextAiringAt: number | null;
  episodesSource: string | null;
  episodeList: EpisodeRow[];
  relations: RelationCard[];
  extras: ExtraFile[];
  otherFiles: FileRef[];
  groups: GroupRef[];
  /** Owned entries of the same franchise in release order (empty if standalone). */
  franchise: FranchiseEntry[];
  trackPref: MediaTrackPref | null;
  anilistStatus?: string | null;
  anilistProgress?: number | null;
  anilistScore?: number | null;
  characters?: CharacterCard[];
  staff?: StaffCard[];
  inStreamLibrary?: boolean;
}

export interface PersonSearchResult {
  staffId: number;
  nameFull: string;
  nameNative: string | null;
  imageUrl: string | null;
  occupations: string[];
}

export interface TorrentCandidate {
  title: string;
  group: string | null;
  resolution: string | null;
  size: number;
  seeders: number;
  leechers: number;
  torrentUrl: string | null;
  magnetUri: string | null;
  infoHash: string | null;
  publishedAt: number | null;
  source: string;
  isBatch: boolean;
  score: number;
  refSubtitleUrl: string | null;
}

export interface TorrentExtension {
  id: string;
  name: string;
  kind: "rss" | "subsplease-json" | "seadex-api" | string;
  url: string;
  enabled: boolean;
  iconUrl?: string | null;
  description?: string | null;
  version?: string | null;
  accuracy?: string | null;
}

export interface TorrentSessionStatus {
  sessionId: string;
  anilistId: number;
  epKey: string;
  title: string;
  fileName: string;
  mode: "stream" | "hybrid" | "download" | string;
  state: "initializing" | "buffering" | "streaming" | "downloading" | "completed" | "error" | string;
  progressPct: number;
  downloadedBytes: number;
  uploadedBytes: number;
  totalBytes: number;
  speedBps: number;
  uploadSpeedBps: number;
  peers: number;
  streamUrl: string | null;
  savePath: string;
  error: string | null;
}

export interface AnilistRemoteEntry {
  anilistId: number;
  status: string;
  progress: number;
  score: number | null;
  updatedAt: number;
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
  logoUrl: string | null;
  logoPath: string | null;
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

export type PlayerKind = "builtin" | "system" | "mpv" | "vlc" | "mpc" | "memento";

export interface DetectedPlayer {
  kind: PlayerKind;
  path: string;
}

export interface MpvStatus {
  installed: boolean;
  path: string | null;
  hasWinget: boolean;
  hasChoco: boolean;
  packageManager: string | null;
}

export interface InstallMpvResult {
  success: boolean;
  method: string;
  message: string;
  path: string | null;
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

export interface BackupSummary {
  path: string;
  /** Unix seconds. */
  createdAt: number;
  appVersion: string;
  format: number;
  settings: number;
  libraries: { name: string; folders: number }[];
  watched: number;
  showsWatched: number;
  progress: number;
  trackPrefs: number;
  manualMatches: number;
}

export interface ImportOptions {
  settings: boolean;
  libraries: boolean;
  history: boolean;
  trackPrefs: boolean;
  matches: boolean;
}

export interface ImportReport {
  settings: number;
  librariesAdded: number;
  foldersAdded: number;
  watchedAdded: number;
  progressAdded: number;
  trackPrefs: number;
  matches: number;
  matchesSkipped: number;
}

export interface SafetyCopy {
  path: string;
  /** Unix seconds of the last write; null if the file doesn't exist yet. */
  modified: number | null;
}

export interface Diagnostics {
  /** Plain-text report for bug reports. */
  report: string;
  logPath: string;
  players: DetectedPlayer[];
  logTail: string[];
}

export interface AnilistAccount {
  connected: boolean;
  userId: number | null;
  userName: string | null;
  avatarUrl: string | null;
  lastSyncAt: number | null;
  autoScrobble: boolean;
  queuedCount: number;
  trackedCount: number;
  clientId: string;
}

export interface AnilistSyncReport {
  pulledSeries: number;
  pulledEpisodes: number;
  pushedSeries: number;
  flushedQueue: number;
  totalRemote: number;
}

export interface SeasonalItem {
  anilistId: number;
  linkId: number;
  titleRomaji: string | null;
  titleEnglish: string | null;
  titleNative: string | null;
  format: string | null;
  status: string | null;
  episodes: number | null;
  duration: number | null;
  season: string | null;
  seasonYear: number | null;
  averageScore: number | null;
  genres: string[];
  studios: string[];
  startDate: string | null;
  coverUrl: string | null;
  coverPath: string | null;
  coverColor: string | null;
  bannerUrl: string | null;
  nextAiringEpisode: number | null;
  nextAiringAt: number | null;
  inLibrary: boolean;
  ownedCount: number;
  watchedCount: number;
  anilistStatus: string | null;
  anilistProgress: number | null;
}

export interface SeasonalCalendar {
  season: string;
  year: number;
  fetchedAt: number;
  items: SeasonalItem[];
}

export interface AiringNotification {
  anilistId: number;
  title: string;
  episode: string | number;
  kind?: "episode" | "jimaku_sub" | "episode_and_sub" | string;
  message: string;
}

export interface TrackItem {
  id: number;
  trackType: string;
  title: string | null;
  lang: string | null;
  codec: string | null;
  selected: boolean;
  external: boolean;
}

export interface ChapterItem {
  title: string;
  time: number;
}

export interface BuiltinPlayerState {
  active: boolean;
  backend: "mpv" | "html5" | string;
  anilistId: number;
  epKey: string;
  path: string;
  streamUrl: string;
  transcodeBaseUrl: string | null;
  showTitle: string;
  episodeTitle: string;
  paused: boolean;
  position: number;
  duration: number;
  volume: number;
  muted: boolean;
  speed: number;
  aid: number | null;
  sid: number | null;
  subText: string;
  subOffsetSec: number;
  audioTracks: TrackItem[];
  subTracks: TrackItem[];
  chapters: ChapterItem[];
  opStart: number | null;
  opEnd: number | null;
  edStart: number | null;
  edEnd: number | null;
  hasNextEpisode: boolean;
  externalSubPath: string | null;
}

export type BuiltinCommand =
  | { type: "play_pause" }
  | { type: "pause" }
  | { type: "resume" }
  | { type: "seek"; position: number }
  | { type: "seek_relative"; delta: number }
  | { type: "set_volume"; volume: number }
  | { type: "toggle_mute" }
  | { type: "set_speed"; speed: number }
  | { type: "set_audio_track"; id: number }
  | { type: "set_sub_track"; id: number }
  | { type: "add_sub_offset"; delta: number }
  | { type: "load_sub_file"; path: string }
  | { type: "next_episode" }
  | { type: "open_external_mpv" }
  | { type: "html5_tick"; position: number; duration: number; paused: boolean; ended: boolean }
  | { type: "sync_bounds" }
  | { type: "stop" };

export interface DictionaryInfo {
  id: number;
  title: string;
  revision: string;
  termCount: number;
  kanjiCount: number;
  metaCount: number;
  importedAt: number;
}

export interface FuriganaSegment {
  text: string;
  reading: string | null;
}

export interface PitchAccent {
  reading: string;
  downstep: number;
}

export interface TermEntry {
  expression: string;
  reading: string;
  furigana: FuriganaSegment[];
  matchedText: string;
  matchLen: number;
  deinflectionReasons: string[];
  definitionTags: string[];
  termTags: string[];
  rules: string[];
  glossaries: string[];
  dictionaryTitle: string;
  score: number;
  frequencies: string[];
  pitches: PitchAccent[];
}

export interface KanjiEntry {
  character: string;
  onyomi: string[];
  kunyomi: string[];
  meanings: string[];
  tags: string[];
  stats: Record<string, string>;
  dictionaryTitle: string;
}

export interface LookupResult {
  matchedText: string;
  matchLen: number;
  terms: TermEntry[];
  kanji: KanjiEntry[];
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
  checkMpvStatus: () => invoke<MpvStatus>("check_mpv_status"),
  installMpv: () => invoke<InstallMpvResult>("install_mpv"),
  playEpisode: (anilistId: number, epKey: string, path: string) =>
    invoke<void>("play_episode", { anilistId, epKey, path }),
  getUpNext: (limit = 6) => invoke<UpNextItem[]>("get_up_next", { limit }),
  getNewEpisodes: (limit = 6) => invoke<UpNextItem[]>("get_new_episodes", { limit }),
  getMediaTrackPref: (anilistId: number) => invoke<MediaTrackPref | null>("get_media_track_pref", { anilistId }),
  setMediaTrackPref: (anilistId: number, pref: MediaTrackPref | null) =>
    invoke<void>("set_media_track_pref", { anilistId, pref }),
  rebuildMetadata: () => invoke<void>("rebuild_metadata"),
  exportBackup: (path: string) => invoke<BackupSummary>("export_backup", { path }),
  previewBackup: (path: string) => invoke<BackupSummary>("preview_backup", { path }),
  importBackup: (path: string, options: ImportOptions) => invoke<ImportReport>("import_backup", { path, options }),
  backupStatus: () => invoke<SafetyCopy[]>("backup_status"),
  getDiagnostics: () => invoke<Diagnostics>("get_diagnostics"),
  getAnilistAccount: () => invoke<AnilistAccount>("get_anilist_account"),
  anilistUpdateEntry: (opts: { anilistId: number; status?: string | null; progress?: number | null; score?: number | null }) =>
    invoke<AnilistRemoteEntry>("anilist_update_entry", {
      anilistId: opts.anilistId,
      status: opts.status ?? null,
      progress: opts.progress ?? null,
      score: opts.score ?? null,
    }),
  anilistStartOauth: (clientId?: string) => invoke<string>("anilist_start_oauth", { clientId: clientId ?? null }),
  anilistLoginToken: (token: string) => invoke<AnilistAccount>("anilist_login_token", { token }),
  anilistDisconnect: () => invoke<AnilistAccount>("anilist_disconnect"),
  anilistSyncNow: () => invoke<AnilistSyncReport>("anilist_sync_now"),
  getSeasonalCalendar: (opts?: { season?: string; year?: number; refresh?: boolean }) =>
    invoke<SeasonalCalendar>("get_seasonal_calendar", {
      season: opts?.season ?? null,
      year: opts?.year ?? null,
      refresh: opts?.refresh ?? false,
    }),
  getPersonWorks: (staffId: number) => invoke<PersonDetail>("get_person_works", { staffId }),
  previewSubtitleMatches: (anilistId: number, defaultLang = "ja") =>
    invoke<SubtitleMatchPreview>("preview_subtitle_matches", { anilistId, defaultLang }),
  applySubtitleMatches: (opts: {
    anilistId: number;
    mode: "link" | "rename";
    items: SubtitleMatchItem[];
    autoRetime: boolean;
  }) => invoke<SubtitleApplyReport>("apply_subtitle_matches", opts),
  retimeEpisodeSubtitle: (opts: { anilistId: number; epKey: string; subtitlePath: string }) =>
    invoke<RetimeReport>("retime_episode_subtitle", opts),
  removeLinkedSubtitle: (id: number) => invoke<void>("remove_linked_subtitle", { id }),
  jimakuSearch: (anilistId: number, episode?: number | null) =>
    invoke<JimakuSearchResponse>("jimaku_search", { anilistId, episode: episode ?? null }),
  jimakuDownload: (opts: {
    anilistId: number;
    epKey: string;
    fileName: string;
    fileUrl: string;
    autoRetime: boolean;
  }) => invoke<JimakuDownloadResult>("jimaku_download", opts),
  toggleStreamLibrary: (anilistId: number) => invoke<boolean>("toggle_stream_library", { anilistId }),
  browseCatalog: (opts?: {
    search?: string | null;
    genre?: string | null;
    format?: string | null;
    status?: string | null;
    season?: string | null;
    year?: number | null;
    sort?: string | null;
    page?: number | null;
  }) =>
    invoke<SeasonalItem[]>("browse_catalog", {
      search: opts?.search ?? null,
      genre: opts?.genre ?? null,
      format: opts?.format ?? null,
      status: opts?.status ?? null,
      season: opts?.season ?? null,
      year: opts?.year ?? null,
      sort: opts?.sort ?? null,
      page: opts?.page ?? 1,
    }),
  searchPeople: (query: string) => invoke<PersonSearchResult[]>("search_people", { query }),
  searchEpisodeTorrents: (anilistId: number, episode: number, customQuery?: string | null) =>
    invoke<TorrentCandidate[]>("search_episode_torrents", {
      anilistId,
      episode,
      customQuery: customQuery ?? null,
    }),
  startTorrentSession: (opts: {
    anilistId: number;
    epKey: string;
    episodeNum?: number | null;
    candidate: TorrentCandidate;
    modeOverride?: "stream" | "hybrid" | "download" | null;
  }) =>
    invoke<TorrentSessionStatus>("start_torrent_session", {
      anilistId: opts.anilistId,
      epKey: opts.epKey,
      episodeNum: opts.episodeNum ?? null,
      candidate: opts.candidate,
      modeOverride: opts.modeOverride ?? null,
    }),
  listTorrentSessions: () => invoke<TorrentSessionStatus[]>("list_torrent_sessions"),
  cancelTorrentSession: (sessionId: string) => invoke<void>("cancel_torrent_session", { sessionId }),
  listTorrentExtensions: () => invoke<TorrentExtension[]>("list_torrent_extensions"),
  importTorrentExtension: (input: string) =>
    invoke<TorrentExtension[]>("import_torrent_extension", { input }),
  toggleTorrentExtension: (id: string, enabled: boolean) =>
    invoke<TorrentExtension[]>("toggle_torrent_extension", { id, enabled }),
  removeTorrentExtension: (id: string) =>
    invoke<TorrentExtension[]>("remove_torrent_extension", { id }),
  builtinPlayerGetState: () => invoke<BuiltinPlayerState>("builtin_player_get_state"),
  builtinPlayerCommand: (command: BuiltinCommand) =>
    invoke<BuiltinPlayerState>("builtin_player_command", { command }),
  lookupJapaneseAt: (text: string, charOffset: number) =>
    invoke<LookupResult>("lookup_japanese_at", { text, charOffset }),
  listDictionaries: () => invoke<DictionaryInfo[]>("list_dictionaries"),
  importYomitanDictionary: (zipPath: string) =>
    invoke<DictionaryInfo>("import_yomitan_dictionary", { zipPath }),
  downloadRecommendedDictionaries: () =>
    invoke<DictionaryInfo[]>("download_recommended_dictionaries"),
  removeDictionary: (id: number) => invoke<void>("remove_dictionary", { id }),
};

export const events = {
  onLibraryChanged: (cb: () => void): Promise<UnlistenFn> => listen("library-changed", () => cb()),
  onScanProgress: (cb: (p: ScanProgress) => void): Promise<UnlistenFn> =>
    listen<ScanProgress>("scan-progress", (e) => cb(e.payload)),
  onScanFinished: (cb: () => void): Promise<UnlistenFn> => listen("scan-finished", () => cb()),
  onScanError: (cb: (msg: string) => void): Promise<UnlistenFn> => listen<string>("scan-error", (e) => cb(e.payload)),
  onPlayback: (cb: (p: PlaybackEvent) => void): Promise<UnlistenFn> =>
    listen<PlaybackEvent>("playback", (e) => cb(e.payload)),
  onMpvInstallProgress: (cb: (msg: string) => void): Promise<UnlistenFn> =>
    listen<string>("mpv-install-progress", (e) => cb(e.payload)),
  onAnilistChanged: (cb: (acct: AnilistAccount) => void): Promise<UnlistenFn> =>
    listen<AnilistAccount>("anilist-changed", (e) => cb(e.payload)),
  onAiringNotification: (cb: (n: AiringNotification) => void): Promise<UnlistenFn> =>
    listen<AiringNotification>("airing-notification", (e) => cb(e.payload)),
  onTorrentProgress: (cb: (s: TorrentSessionStatus) => void): Promise<UnlistenFn> =>
    listen<TorrentSessionStatus>("torrent-progress", (e) => cb(e.payload)),
  onBuiltinPlayerState: (cb: (s: BuiltinPlayerState) => void): Promise<UnlistenFn> =>
    listen<BuiltinPlayerState>("builtin-player-state", (e) => cb(e.payload)),
  onDictionaryProgress: (cb: (p: { stage: string; message: string }) => void): Promise<UnlistenFn> =>
    listen<{ stage: string; message: string }>("dictionary-progress", (e) => cb(e.payload)),
};

/** Prefer the locally cached image (works offline), fall back to the remote URL. */
export function img(localPath: string | null | undefined, remote: string | null | undefined): string | null {
  if (localPath) return convertFileSrc(localPath);
  return remote ?? null;
}
