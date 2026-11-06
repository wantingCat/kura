<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { api, type BuiltinPlayerState, type LookupResult } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import YomitanPopup from "./YomitanPopup.svelte";

  let {
    playerState,
  }: {
    playerState: BuiltinPlayerState;
  } = $props();

  let controlsVisible = $state(true);
  let hideTimer: ReturnType<typeof setTimeout> | null = null;
  let activeMenu = $state<"none" | "tracks" | "speed">("none");

  // Yomitan lookup state (active while paused)
  let lookupResult = $state<LookupResult | null>(null);
  let highlightStart = $state<number>(-1);
  let highlightEnd = $state<number>(-1);
  let popupAnchorX = $state<number>(640);
  let popupAnchorY = $state<number>(560);
  let popupPinned = $state<boolean>(false);
  let lookupReqCounter = 0;
  let lookupDwellTimer: ReturnType<typeof setTimeout> | null = null;
  let trackedForms = $state<string[]>([]);

  async function refreshTrackedForms() {
    try {
      trackedForms = await api.getTrackedSurfaceForms();
    } catch {
      // ignore
    }
  }

  $effect(() => {
    if (playerState.active) {
      void refreshTrackedForms();
    }
  });

  function formatImmersionBadge(sec: number): string {
    const mins = Math.max(0, Math.floor(sec / 60));
    if (mins >= 60) {
      const h = Math.floor(mins / 60);
      const m = mins % 60;
      return m > 0 ? `${h}h ${m}m` : `${h}h`;
    }
    return `${mins}m`;
  }

  // In-window HTML5 video surface binding + automatic live mpv transcoding fallback
  let html5Video = $state<HTMLVideoElement | null>(null);
  let useTranscode = $state(false);
  let transcodeVideoCodec = $state<"copy" | "h264">("copy");
  let transcodeStartOffset = $state(0);
  let transcodeRev = $state(0);
  let initialSeekDone = $state(false);
  let pendingSeekOnLoad = $state<number | null>(null);
  let lastLoadedPath = $state("");
  let isBuffering = $state(false);

  // Smooth 60fps scrubber state
  let isScrubbing = $state(false);
  let scrubValue = $state(0);
  let hoverPct = $state(0);
  let hoverTime = $state<number | null>(null);
  let hoverChapter = $state<string | null>(null);
  let transcodeSeekTimer: ReturnType<typeof setTimeout> | null = null;

  let displayPosition = $derived(isScrubbing ? scrubValue : playerState.position);

  // Active torrent session HUD stats
  let activeTorrentSession = $derived.by(() => {
    return (
      app.torrentSessions.find(
        (s) => s.anilistId === playerState.anilistId && s.epKey === playerState.epKey
      ) ??
      app.torrentSessions[0] ??
      null
    );
  });

  function formatBytes(bytes: number): string {
    if (!bytes || bytes <= 0) return "0 KB";
    const mb = bytes / (1024 * 1024);
    if (mb >= 1024) return `${(mb / 1024).toFixed(2)} GB`;
    if (mb >= 1) return `${mb.toFixed(1)} MB`;
    return `${(bytes / 1024).toFixed(0)} KB`;
  }

  function isUnsupportedAudioCodec(codecRaw: string | null | undefined): boolean {
    const codec = (codecRaw ?? "").toLowerCase();
    return /eac3|ac3|dts|truehd|mlp/.test(codec);
  }

  $effect(() => {
    const curPath = `${playerState.anilistId}:${playerState.epKey}:${playerState.path}`;
    if (curPath !== lastLoadedPath) {
      lastLoadedPath = curPath;
      useTranscode = false;
      transcodeVideoCodec = "copy";
      transcodeStartOffset = playerState.position > 1.0 ? playerState.position : 0;
      transcodeRev = 0;
      initialSeekDone = false;
      pendingSeekOnLoad = null;
      isBuffering = false;
      isScrubbing = false;
      clearLookup();
    }
  });

  // Engage live mpv audio remux (`vc=copy`) ONLY when the selected audio track's codec
  // is unsupported by Chromium/WebView2 (E-AC3, AC3, DTS, TrueHD, MLP).
  // Dual-Audio AAC/Opus/FLAC tracks (`aid > 1`) are served natively over HTTP 206 byte-ranges
  // using zero-transcode Matroska EBML header track masking (`?aid=N`) for instant <10ms seeking!
  $effect(() => {
    if (
      !useTranscode &&
      playerState.transcodeBaseUrl &&
      playerState.audioTracks.length > 0
    ) {
      const selectedTrack =
        playerState.audioTracks.find((t) => t.selected) ?? playerState.audioTracks[0];
      if (isUnsupportedAudioCodec(selectedTrack?.codec)) {
        transcodeVideoCodec = "copy";
        transcodeStartOffset = playerState.position > 1.0 ? playerState.position : 0;
        useTranscode = true;
      }
    }
  });

  let activeVideoSrc = $derived.by(() => {
    const aid = playerState.aid ?? 1;
    if (useTranscode && playerState.transcodeBaseUrl) {
      return `${playerState.transcodeBaseUrl}?start=${transcodeStartOffset.toFixed(1)}&aid=${aid}&vc=${transcodeVideoCodec}&rev=${transcodeRev}`;
    }
    const base =
      playerState.streamUrl ||
      (playerState.path.startsWith("http://") || playerState.path.startsWith("https://")
        ? playerState.path
        : "");
    if (!base) return "";
    if (aid > 1) {
      const sep = base.includes("?") ? "&" : "?";
      return `${base}${sep}aid=${aid}`;
    }
    return base;
  });

  // Split subtitle text into an array of Unicode codepoints with their character indices
  let subChars = $derived.by(() => {
    const raw = playerState.subText || "";
    return Array.from(raw).map((ch, idx) => ({ ch, idx }));
  });

  // Identify character indices belonging to tracked ('learning') Japanese words
  let trackedCharIndices = $derived.by(() => {
    const indices = new Set<number>();
    const raw = playerState.subText || "";
    if (!raw || trackedForms.length === 0) return indices;
    const codepoints = Array.from(raw);
    for (const form of trackedForms) {
      if (!form) continue;
      const formChars = Array.from(form);
      const fLen = formChars.length;
      if (fLen === 0 || fLen > codepoints.length) continue;
      for (let i = 0; i <= codepoints.length - fLen; i++) {
        let match = true;
        for (let j = 0; j < fLen; j++) {
          if (codepoints[i + j] !== formChars[j]) {
            match = false;
            break;
          }
        }
        if (match) {
          for (let j = 0; j < fLen; j++) {
            indices.add(i + j);
          }
        }
      }
    }
    return indices;
  });

  // Detect active OP / ED skip window
  let activeSkip = $derived.by(() => {
    const pos = displayPosition;
    if (
      playerState.opStart !== null &&
      playerState.opEnd !== null &&
      pos >= playerState.opStart &&
      pos < playerState.opEnd - 1.0
    ) {
      return { label: "Skip Opening", target: playerState.opEnd };
    }
    if (
      playerState.edStart !== null &&
      playerState.edEnd !== null &&
      pos >= playerState.edStart &&
      pos < playerState.edEnd - 1.0
    ) {
      return { label: "Skip Ending", target: playerState.edEnd };
    }
    return null;
  });

  // Automatically clear Yomitan popup and schedule HUD auto-hide when video resumes playing
  $effect(() => {
    if (!playerState.paused) {
      clearLookup();
      bumpControls(true);
    }
  });

  // Sync bounds on window resize
  $effect(() => {
    if (!playerState.active) return;
    const onResize = () => {
      void api.builtinPlayerCommand({ type: "sync_bounds" });
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  });

  function clearLookup() {
    if (lookupDwellTimer) {
      clearTimeout(lookupDwellTimer);
      lookupDwellTimer = null;
    }
    lookupResult = null;
    highlightStart = -1;
    highlightEnd = -1;
    popupPinned = false;
  }

  async function logTopTermLookup(res: LookupResult) {
    const top = res.terms[0];
    if (!top) return;
    try {
      const saved = await api.recordWordLookup({
        expression: top.expression,
        reading: top.reading || top.expression,
        matchedText: top.matchedText || res.matchedText,
        furigana: top.furigana,
        glossaries: top.glossaries,
        pitches: top.pitches,
        frequencies: top.frequencies,
        posTags: [...top.definitionTags, ...top.termTags].join(" "),
        anilistId: playerState.anilistId,
        epKey: playerState.epKey,
        sentence: playerState.subText,
        positionSec: displayPosition,
      });
      if (lookupResult && lookupResult.matchedText === res.matchedText) {
        lookupResult = {
          ...lookupResult,
          terms: lookupResult.terms.map((t, idx) =>
            idx === 0
              ? { ...t, wordStatus: saved.status, lookupCount: saved.lookupCount }
              : t
          ),
        };
      }
      void refreshTrackedForms();
    } catch {
      // ignore
    }
  }

  function bumpControls(forcePlaying = false) {
    controlsVisible = true;
    if (hideTimer) clearTimeout(hideTimer);
    const isPlaying = forcePlaying || !playerState.paused || (html5Video ? !html5Video.paused : false);
    if (isPlaying) {
      hideTimer = setTimeout(() => {
        if (!playerState.paused && activeMenu === "none" && !isScrubbing) {
          controlsVisible = false;
        }
      }, 2400);
    }
  }

  async function triggerLookup(charIdx: number, targetEl: HTMLElement, pin: boolean) {
    if (!playerState.paused) return;
    if (popupPinned && !pin) return;

    const ch = subChars[charIdx]?.ch ?? "";
    if (!ch.trim()) return;

    const rect = targetEl.getBoundingClientRect();
    popupAnchorX = rect.left + rect.width / 2;
    popupAnchorY = rect.top;

    const reqId = ++lookupReqCounter;
    try {
      const res = await api.lookupJapaneseAt(playerState.subText, charIdx);
      if (reqId !== lookupReqCounter || !playerState.paused) return;
      if (res.matchLen > 0 && (res.terms.length > 0 || res.kanji.length > 0)) {
        lookupResult = res;
        highlightStart = charIdx;
        highlightEnd = charIdx + res.matchLen;
        if (lookupDwellTimer) {
          clearTimeout(lookupDwellTimer);
          lookupDwellTimer = null;
        }
        if (pin) {
          popupPinned = true;
          void logTopTermLookup(res);
        } else {
          lookupDwellTimer = setTimeout(() => {
            if (playerState.paused && lookupResult?.matchedText === res.matchedText) {
              void logTopTermLookup(res);
            }
          }, 360);
        }
      } else if (!popupPinned) {
        clearLookup();
      }
    } catch {
      // ignore lookup errors
    }
  }

  function handleCharHover(charIdx: number, e: MouseEvent) {
    if (!playerState.paused || popupPinned) return;
    const el = e.currentTarget as HTMLElement;
    void triggerLookup(charIdx, el, false);
  }

  function handleCharClick(charIdx: number, e: MouseEvent) {
    e.stopPropagation();
    if (!playerState.paused) {
      void togglePlayPause();
      return;
    }
    const el = e.currentTarget as HTMLElement;
    if (popupPinned && charIdx >= highlightStart && charIdx < highlightEnd) {
      clearLookup();
      return;
    }
    void triggerLookup(charIdx, el, true);
  }

  function handleSubtitleMouseLeave() {
    if (!popupPinned) {
      clearLookup();
    }
  }

  async function togglePlayPause() {
    if (html5Video) {
      if (html5Video.paused) {
        await html5Video.play().catch(() => {});
      } else {
        html5Video.pause();
      }
    }
    await api.builtinPlayerCommand({ type: "play_pause" });
    bumpControls();
  }

  function canSeekWithinTranscodeBuffer(v: HTMLVideoElement, targetSec: number): boolean {
    const rel = targetSec - transcodeStartOffset;
    if (rel < 0) return false;
    const buf = v.buffered;
    for (let i = 0; i < buf.length; i++) {
      if (rel >= buf.start(i) - 0.35 && rel <= buf.end(i) - 0.15) {
        return true;
      }
    }
    return false;
  }

  async function seekTo(sec: number) {
    const maxDur = playerState.duration > 0 ? playerState.duration : sec;
    const clamped = Math.max(0, Math.min(sec, maxDur));

    if (useTranscode && playerState.transcodeBaseUrl) {
      if (html5Video && canSeekWithinTranscodeBuffer(html5Video, clamped)) {
        // Target is already in WebView2's media buffer — seek in <10ms without respawning mpv!
        html5Video.currentTime = Math.max(0, clamped - transcodeStartOffset);
      } else {
        if (transcodeSeekTimer) clearTimeout(transcodeSeekTimer);
        transcodeSeekTimer = setTimeout(() => {
          transcodeStartOffset = clamped;
          transcodeRev += 1;
        }, 120);
      }
    } else if (html5Video) {
      html5Video.currentTime = clamped;
    }
    await api.builtinPlayerCommand({ type: "seek", position: clamped });
  }

  async function seekRelative(delta: number) {
    const base = isScrubbing ? scrubValue : playerState.position;
    await seekTo(base + delta);
    bumpControls();
  }

  function onScrubInput(e: Event) {
    const val = Number((e.currentTarget as HTMLInputElement).value);
    isScrubbing = true;
    scrubValue = val;
    if (!useTranscode && html5Video) {
      html5Video.currentTime = val;
    }
    bumpControls();
  }

  async function onScrubCommit(e: Event) {
    const val = Number((e.currentTarget as HTMLInputElement).value);
    isScrubbing = false;
    await seekTo(val);
  }

  function onTimelineMouseMove(e: MouseEvent) {
    if (playerState.duration <= 0) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / Math.max(1, rect.width)));
    hoverPct = pct * 100;
    const t = pct * playerState.duration;
    hoverTime = t;
    let matchedChapter: string | null = null;
    for (const ch of playerState.chapters) {
      if (t >= ch.time) {
        matchedChapter = ch.title;
      } else {
        break;
      }
    }
    hoverChapter = matchedChapter;
  }

  function onTimelineMouseLeave() {
    hoverTime = null;
    hoverChapter = null;
  }

  async function setVolume(vol: number) {
    if (html5Video) {
      html5Video.volume = Math.max(0, Math.min(1, vol / 100));
    }
    await api.builtinPlayerCommand({ type: "set_volume", volume: vol });
  }

  async function toggleMute() {
    if (html5Video) {
      html5Video.muted = !html5Video.muted;
    }
    await api.builtinPlayerCommand({ type: "toggle_mute" });
  }

  async function setSpeed(speed: number) {
    if (html5Video) {
      html5Video.playbackRate = speed;
    }
    await api.builtinPlayerCommand({ type: "set_speed", speed });
    activeMenu = "none";
  }

  async function selectAudioTrack(id: number) {
    const savedPos = html5Video
      ? useTranscode
        ? transcodeStartOffset + html5Video.currentTime
        : html5Video.currentTime
      : playerState.position;
    const targetTrack = playerState.audioTracks.find((t) => t.id === id);
    const needsAudioTc = isUnsupportedAudioCodec(targetTrack?.codec);

    await api.builtinPlayerCommand({ type: "set_audio_track", id });

    if (needsAudioTc || transcodeVideoCodec === "h264") {
      if (playerState.transcodeBaseUrl) {
        transcodeStartOffset = savedPos;
        useTranscode = true;
        transcodeRev += 1;
      }
    } else {
      pendingSeekOnLoad = savedPos;
      useTranscode = false;
    }
    activeMenu = "none";
  }

  async function pickExternalSubtitle() {
    const selected = await openDialog({
      multiple: false,
      filters: [{ name: "Subtitles", extensions: ["srt", "ass", "ssa", "vtt"] }],
    });
    if (typeof selected === "string" && selected) {
      await api.builtinPlayerCommand({ type: "load_sub_file", path: selected });
      app.toast("Loaded external subtitle file");
      activeMenu = "none";
    }
  }

  async function closePlayer() {
    clearLookup();
    if (document.fullscreenElement) {
      await document.exitFullscreen().catch(() => {});
    }
    await api.builtinPlayerCommand({ type: "stop" });
  }

  async function toggleFullscreen() {
    if (!document.fullscreenElement) {
      await document.documentElement.requestFullscreen().catch(() => {});
    } else {
      await document.exitFullscreen().catch(() => {});
    }
    setTimeout(() => {
      void api.builtinPlayerCommand({ type: "sync_bounds" });
    }, 120);
  }

  function formatTime(sec: number): string {
    if (!Number.isFinite(sec) || sec < 0) return "0:00";
    const total = Math.floor(sec);
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const s = total % 60;
    if (h > 0) {
      return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
    }
    return `${m}:${String(s).padStart(2, "0")}`;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (!playerState.active) return;
    const tag = (e.target as HTMLElement | null)?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

    switch (e.key) {
      case " ":
      case "k":
      case "K":
        e.preventDefault();
        void togglePlayPause();
        break;
      case "ArrowLeft":
      case "j":
      case "J":
        e.preventDefault();
        void seekRelative(-5);
        break;
      case "ArrowRight":
      case "l":
      case "L":
        e.preventDefault();
        void seekRelative(5);
        break;
      case "ArrowUp":
        e.preventDefault();
        void setVolume(Math.min(100, playerState.volume + 5));
        break;
      case "ArrowDown":
        e.preventDefault();
        void setVolume(Math.max(0, playerState.volume - 5));
        break;
      case "m":
      case "M":
        e.preventDefault();
        void toggleMute();
        break;
      case "s":
      case "S":
        if (activeSkip) {
          e.preventDefault();
          void seekTo(activeSkip.target);
        }
        break;
      case "f":
      case "F":
        e.preventDefault();
        void toggleFullscreen();
        break;
      case "Escape":
        e.preventDefault();
        if (lookupResult) {
          clearLookup();
        } else if (activeMenu !== "none") {
          activeMenu = "none";
        } else {
          void closePlayer();
        }
        break;
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if playerState.active}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="kura-player-root"
    class:controls-hidden={!controlsVisible && !playerState.paused}
    onmousemove={() => bumpControls()}
    onclick={() => {
      if (popupPinned) {
        clearLookup();
      } else if (activeMenu !== "none") {
        activeMenu = "none";
      } else {
        void togglePlayPause();
      }
    }}
    ondblclick={() => void toggleFullscreen()}
  >
    {#if activeVideoSrc}
      <video
        bind:this={html5Video}
        class="html5-video-surface"
        src={activeVideoSrc}
        autoplay
        playsinline
        onwaiting={() => {
          isBuffering = true;
        }}
        onplaying={() => {
          isBuffering = false;
        }}
        oncanplay={() => {
          isBuffering = false;
        }}
        onloadedmetadata={(e) => {
          const v = e.currentTarget;
          isBuffering = false;
          if ((!useTranscode || transcodeVideoCodec === "copy") && v.videoWidth === 0 && playerState.transcodeBaseUrl) {
            transcodeVideoCodec = "h264";
            transcodeStartOffset = playerState.position > 1.0 ? playerState.position : 0;
            useTranscode = true;
            transcodeRev += 1;
            return;
          }
          if (!useTranscode) {
            if (pendingSeekOnLoad !== null && pendingSeekOnLoad > 0.5) {
              v.currentTime = pendingSeekOnLoad;
              pendingSeekOnLoad = null;
              initialSeekDone = true;
            } else if (!initialSeekDone && playerState.position > 1.0) {
              initialSeekDone = true;
              v.currentTime = playerState.position;
            }
          }
          v.volume = Math.max(0, Math.min(1, playerState.volume / 100));
          v.muted = playerState.muted;
          v.playbackRate = playerState.speed;
        }}
        onerror={() => {
          if ((!useTranscode || transcodeVideoCodec === "copy") && playerState.transcodeBaseUrl) {
            transcodeVideoCodec = "h264";
            transcodeStartOffset = playerState.position > 1.0 ? playerState.position : 0;
            useTranscode = true;
            transcodeRev += 1;
          }
        }}
        ontimeupdate={(e) => {
          const v = e.currentTarget;
          const isRemoteStream =
            playerState.path.startsWith("http://") || playerState.path.startsWith("https://");
          if (
            !isRemoteStream &&
            !useTranscode &&
            playerState.transcodeBaseUrl &&
            !v.paused &&
            v.currentTime > 2.5
          ) {
            const audioBytes = (v as unknown as { webkitAudioDecodedByteCount?: number }).webkitAudioDecodedByteCount;
            if (typeof audioBytes === "number" && audioBytes === 0 && playerState.audioTracks.length > 0) {
              transcodeVideoCodec = "copy";
              transcodeStartOffset = v.currentTime > 1.0 ? v.currentTime : 0;
              useTranscode = true;
              return;
            }
          }
          const pos = useTranscode ? transcodeStartOffset + v.currentTime : v.currentTime;
          const dur =
            !useTranscode && Number.isFinite(v.duration) && v.duration > 0
              ? v.duration
              : playerState.duration;
          void api.builtinPlayerCommand({
            type: "html5_tick",
            position: pos,
            duration: dur,
            paused: v.paused,
            ended: v.ended,
          });
        }}
        onplay={(e) => {
          const v = e.currentTarget;
          const pos = useTranscode ? transcodeStartOffset + v.currentTime : v.currentTime;
          const dur =
            !useTranscode && Number.isFinite(v.duration) && v.duration > 0
              ? v.duration
              : playerState.duration;
          void api.builtinPlayerCommand({
            type: "html5_tick",
            position: pos,
            duration: dur,
            paused: false,
            ended: false,
          });
        }}
        onpause={(e) => {
          const v = e.currentTarget;
          const pos = useTranscode ? transcodeStartOffset + v.currentTime : v.currentTime;
          const dur =
            !useTranscode && Number.isFinite(v.duration) && v.duration > 0
              ? v.duration
              : playerState.duration;
          void api.builtinPlayerCommand({
            type: "html5_tick",
            position: pos,
            duration: dur,
            paused: true,
            ended: false,
          });
        }}
      >
        <track kind="captions" />
      </video>
    {/if}

    {#if isBuffering && !playerState.paused}
      <div class="buffering-indicator" aria-hidden="true">
        <div class="buffering-ring"></div>
      </div>
    {/if}

    <!-- Top Bar -->
    <header
      class="player-top-bar"
      class:visible={controlsVisible || playerState.paused}
      onclick={(e) => e.stopPropagation()}
    >
      <div class="top-left-group">
        <button type="button" class="hud-btn back-btn" onclick={closePlayer} title="Back (Esc)" aria-label="Exit Player">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 12H5"/><path d="m12 19-7-7 7-7"/></svg>
        </button>
        <div class="title-stack">
          <div class="show-title">{playerState.showTitle}</div>
          <div class="ep-title">{playerState.episodeTitle}</div>
        </div>
      </div>

      <div class="top-right-badges">
        <div class="immersion-hud-pill" title="Active Japanese immersion time tracked today">
          <span class="im-dot"></span>
          <span>Today: {formatImmersionBadge(playerState.todayImmersionSec ?? 0)}</span>
        </div>
        {#if activeTorrentSession}
          <div class="torrent-hud-pill" title={activeTorrentSession.fileName}>
            <span class="th-prog">
              {activeTorrentSession.progressPct.toFixed(1)}%
              <span class="th-dim">
                ({formatBytes(activeTorrentSession.downloadedBytes)} / {formatBytes(activeTorrentSession.totalBytes)})
              </span>
            </span>
            <span class="th-sep">·</span>
            <span class="th-dl">↓ {formatBytes(activeTorrentSession.speedBps)}/s</span>
            <span class="th-sep">·</span>
            <span class="th-ul">↑ {formatBytes(activeTorrentSession.uploadSpeedBps ?? 0)}/s</span>
            <span class="th-sep">·</span>
            <span class="th-peers">{activeTorrentSession.peers}p</span>
          </div>
        {/if}
      </div>
    </header>

    <!-- Skip OP / ED Floating Button -->
    {#if activeSkip}
      <div class="skip-btn-wrap" onclick={(e) => e.stopPropagation()}>
        <button type="button" class="skip-op-ed-btn" onclick={() => activeSkip && seekTo(activeSkip.target)}>
          <span>{activeSkip.label}</span>
          <span class="skip-key">S</span>
        </button>
      </div>
    {/if}

    <!-- Interactive Subtitle Stage -->
    {#if subChars.length > 0}
      <div
        class="subtitle-stage"
        class:interactive={playerState.paused}
        onmouseleave={handleSubtitleMouseLeave}
        onclick={(e) => e.stopPropagation()}
      >
        <div class="subtitle-box">
          {#each subChars as item (item.idx)}
            {#if item.ch === "\n"}
              <br />
            {:else}
              <span
                class="sub-char"
                class:tracked={trackedCharIndices.has(item.idx)}
                class:highlighted={playerState.paused &&
                  item.idx >= highlightStart &&
                  item.idx < highlightEnd}
                onmouseenter={(e) => handleCharHover(item.idx, e)}
                onclick={(e) => handleCharClick(item.idx, e)}
              >{item.ch}</span>
            {/if}
          {/each}
        </div>
      </div>
    {/if}

    <!-- Yomitan Dictionary Popup (Only when paused & word matched) -->
    {#if playerState.paused && lookupResult}
      <YomitanPopup
        result={lookupResult}
        anchorX={popupAnchorX}
        anchorY={popupAnchorY}
        pinned={popupPinned}
        anilistId={playerState.anilistId}
        epKey={playerState.epKey}
        sentence={playerState.subText}
        positionSec={displayPosition}
        onStatusChanged={refreshTrackedForms}
        onClose={clearLookup}
      />
    {/if}

    <!-- Bottom Controls & Scrubber -->
    <footer
      class="player-bottom-bar"
      class:visible={controlsVisible || playerState.paused}
      onclick={(e) => e.stopPropagation()}
    >
      <!-- Timeline Scrubber with OP/ED, Chapter Markers, & Hover Preview -->
      <div
        class="timeline-row"
        onmousemove={onTimelineMouseMove}
        onmouseleave={onTimelineMouseLeave}
      >
        {#if hoverTime !== null && playerState.duration > 0}
          <div class="timeline-tooltip" style="left: {Math.max(4, Math.min(96, hoverPct))}%;">
            <span class="tt-time">{formatTime(hoverTime)}</span>
            {#if hoverChapter}
              <span class="tt-chapter">{hoverChapter}</span>
            {/if}
          </div>
        {/if}
        <div class="scrubber-track">
          {#if playerState.duration > 0}
            {#if playerState.opStart !== null && playerState.opEnd !== null}
              <div
                class="segment-marker op"
                style="left: {(playerState.opStart / playerState.duration) * 100}%; width: {((playerState.opEnd - playerState.opStart) / playerState.duration) * 100}%;"
                title="Opening"
              ></div>
            {/if}
            {#if playerState.edStart !== null && playerState.edEnd !== null}
              <div
                class="segment-marker ed"
                style="left: {(playerState.edStart / playerState.duration) * 100}%; width: {((playerState.edEnd - playerState.edStart) / playerState.duration) * 100}%;"
                title="Ending"
              ></div>
            {/if}
            {#each playerState.chapters as ch}
              <div
                class="chapter-tick"
                style="left: {(ch.time / playerState.duration) * 100}%;"
                title={ch.title}
              ></div>
            {/each}
            <div
              class="scrubber-fill"
              style="width: {Math.min(100, (displayPosition / playerState.duration) * 100)}%;"
            >
              <div class="scrubber-thumb"></div>
            </div>
          {/if}
          <input
            type="range"
            class="scrubber-input"
            min="0"
            max={playerState.duration > 0 ? playerState.duration : 100}
            step="0.1"
            value={displayPosition}
            oninput={onScrubInput}
            onchange={onScrubCommit}
          />
        </div>
      </div>

      <div class="controls-row">
        <div class="controls-left">
          <button type="button" class="ctrl-btn primary" onclick={togglePlayPause} title="Play / Pause (Space)">
            {#if playerState.paused}
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><polygon points="6 4 20 12 6 20 6 4"/></svg>
            {:else}
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/></svg>
            {/if}
          </button>
          <button type="button" class="ctrl-btn" onclick={() => seekRelative(-5)} title="Rewind 5s (← / J)">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>
            <span>5s</span>
          </button>
          <button type="button" class="ctrl-btn" onclick={() => seekRelative(5)} title="Forward 5s (→ / L)">
            <span>5s</span>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/></svg>
          </button>

          {#if playerState.hasNextEpisode}
            <button
              type="button"
              class="ctrl-btn"
              onclick={() => api.builtinPlayerCommand({ type: "next_episode" })}
              title="Next Episode"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><polygon points="5 4 15 12 5 20 5 4"/><line x1="19" y1="5" x2="19" y2="19" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"/></svg>
              <span>Next</span>
            </button>
          {/if}

          <div class="volume-group">
            <button type="button" class="ctrl-btn icon-only" onclick={toggleMute} title="Mute (M)">
              {#if playerState.muted || playerState.volume === 0}
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="23" y1="9" x2="17" y2="15"/><line x1="17" y1="9" x2="23" y2="15"/></svg>
              {:else}
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/></svg>
              {/if}
            </button>
            <input
              type="range"
              class="vol-slider"
              min="0"
              max="100"
              step="1"
              value={playerState.muted ? 0 : playerState.volume}
              oninput={(e) => void setVolume(Number((e.currentTarget as HTMLInputElement).value))}
            />
          </div>

          <div class="time-readout">
            <span class="cur">{formatTime(displayPosition)}</span>
            <span class="sep">/</span>
            <span class="dur">{formatTime(playerState.duration)}</span>
          </div>
        </div>

        <div class="controls-right">
          <!-- Subtitle Delay Nudge -->
          <div class="sub-delay-pill" title="Subtitle Timing Offset">
            <button
              type="button"
              onclick={() => api.builtinPlayerCommand({ type: "add_sub_offset", delta: -0.1 })}
            >−</button>
            <span>Sub {playerState.subOffsetSec >= 0 ? `+${playerState.subOffsetSec.toFixed(1)}` : playerState.subOffsetSec.toFixed(1)}s</span>
            <button
              type="button"
              onclick={() => api.builtinPlayerCommand({ type: "add_sub_offset", delta: 0.1 })}
            >+</button>
          </div>

          <!-- Audio & Subtitle Tracks Menu -->
          <div class="menu-anchor">
            <button
              type="button"
              class="ctrl-btn"
              class:active={activeMenu === "tracks"}
              onclick={() => (activeMenu = activeMenu === "tracks" ? "none" : "tracks")}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="20" height="16" x="2" y="4" rx="2"/><path d="M7 15h4M15 15h2M7 11h2M13 11h4"/></svg>
              <span>Tracks</span>
            </button>
            {#if activeMenu === "tracks"}
              <div class="popover-menu tracks-menu">
                <div class="menu-section">
                  <div class="menu-heading">
                    <span>Subtitles</span>
                    <button type="button" class="mini-action" onclick={pickExternalSubtitle}>
                      + Load File
                    </button>
                  </div>
                  <button
                    type="button"
                    class="menu-item"
                    class:selected={playerState.sid === null}
                    onclick={() => {
                      void api.builtinPlayerCommand({ type: "set_sub_track", id: 0 });
                      activeMenu = "none";
                    }}
                  >
                    <span>Off</span>
                  </button>
                  {#each playerState.subTracks as tr}
                    <button
                      type="button"
                      class="menu-item"
                      class:selected={tr.selected}
                      onclick={() => {
                        void api.builtinPlayerCommand({ type: "set_sub_track", id: tr.id });
                        activeMenu = "none";
                      }}
                    >
                      <span>#{tr.id} {tr.title || tr.lang || "Subtitle"}</span>
                      {#if tr.lang}<span class="lang-tag">{tr.lang}</span>{/if}
                    </button>
                  {/each}
                </div>

                {#if playerState.audioTracks.length > 0}
                  <div class="menu-section">
                    <div class="menu-heading">Audio Track</div>
                    {#each playerState.audioTracks as tr}
                      <button
                        type="button"
                        class="menu-item"
                        class:selected={tr.selected}
                        onclick={() => selectAudioTrack(tr.id)}
                      >
                        <span>#{tr.id} {tr.title || tr.lang || "Audio"}</span>
                        {#if tr.lang}<span class="lang-tag">{tr.lang}</span>{/if}
                      </button>
                    {/each}
                  </div>
                {/if}
              </div>
            {/if}
          </div>

          <!-- Playback Speed Menu -->
          <div class="menu-anchor">
            <button
              type="button"
              class="ctrl-btn"
              class:active={activeMenu === "speed"}
              onclick={() => (activeMenu = activeMenu === "speed" ? "none" : "speed")}
            >
              <span>{playerState.speed}×</span>
            </button>
            {#if activeMenu === "speed"}
              <div class="popover-menu speed-menu">
                {#each [0.75, 1.0, 1.25, 1.5, 2.0] as sp}
                  <button
                    type="button"
                    class="menu-item"
                    class:selected={Math.abs(playerState.speed - sp) < 0.01}
                    onclick={() => setSpeed(sp)}
                  >
                    <span>{sp}×</span>
                  </button>
                {/each}
              </div>
            {/if}
          </div>

          <button type="button" class="ctrl-btn icon-only" onclick={toggleFullscreen} title="Fullscreen (F)">
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/></svg>
          </button>
        </div>
      </div>
    </footer>
  </div>
{/if}

<style>
  .kura-player-root {
    position: fixed;
    inset: 0;
    z-index: 9000;
    background: #000;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    user-select: none;
    font-family: var(--font-body, "Plus Jakarta Sans", system-ui, sans-serif);
  }

  .kura-player-root.controls-hidden {
    cursor: none;
  }

  .html5-video-surface {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    background: #000;
    z-index: -1;
    pointer-events: none;
  }

  .buffering-indicator {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none;
    z-index: 9100;
  }

  .buffering-ring {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    border: 3px solid rgba(255, 255, 255, 0.14);
    border-top-color: #f97316;
    animation: kura-spin 0.75s linear infinite;
  }

  @keyframes kura-spin {
    to {
      transform: rotate(360deg);
    }
  }

  .player-top-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 20px 28px 34px;
    background: linear-gradient(180deg, rgba(8, 8, 12, 0.88) 0%, rgba(8, 8, 12, 0.42) 60%, transparent 100%);
    opacity: 0;
    transform: translateY(-6px);
    transition: opacity 180ms ease, transform 180ms ease;
    pointer-events: none;
  }

  .player-top-bar.visible {
    opacity: 1;
    transform: translateY(0);
    pointer-events: auto;
  }

  .top-left-group {
    display: flex;
    align-items: center;
    gap: 14px;
    min-width: 0;
    flex: 1;
  }

  .hud-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: rgba(15, 15, 20, 0.78);
    backdrop-filter: blur(14px);
    border: 1px solid rgba(255, 255, 255, 0.1);
    color: #f4f4f6;
    font-size: 12.5px;
    font-weight: 600;
    padding: 7px 13px;
    border-radius: 8px;
    cursor: pointer;
    transition: all 140ms ease;
    white-space: nowrap;
  }

  .hud-btn:hover {
    background: rgba(249, 115, 22, 0.22);
    border-color: rgba(249, 115, 22, 0.55);
    color: #fff;
  }

  .back-btn {
    width: 36px;
    height: 36px;
    padding: 0;
    justify-content: center;
    border-radius: 10px;
    flex-shrink: 0;
  }

  .title-stack {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .show-title {
    font-size: 14.5px;
    font-weight: 700;
    letter-spacing: -0.01em;
    color: #fff;
    text-shadow: 0 2px 8px rgba(0, 0, 0, 0.85);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ep-title {
    font-size: 12px;
    color: rgba(244, 244, 246, 0.68);
    text-shadow: 0 1px 6px rgba(0, 0, 0, 0.85);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .top-right-badges {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .immersion-hud-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    padding: 6px 12px;
    border-radius: 99px;
    background: rgba(12, 12, 16, 0.84);
    backdrop-filter: blur(12px);
    border: 1px solid rgba(244, 162, 97, 0.35);
    color: #f7ede2;
    white-space: nowrap;
  }

  .im-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #f4a261;
    box-shadow: 0 0 8px rgba(244, 162, 97, 0.8);
  }

  .torrent-hud-pill {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 11.5px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    padding: 6px 12px;
    border-radius: 99px;
    background: rgba(12, 12, 16, 0.84);
    backdrop-filter: blur(12px);
    border: 1px solid rgba(255, 255, 255, 0.1);
    color: #f4f4f6;
    white-space: nowrap;
  }

  .th-prog {
    color: #ff7a59;
    font-weight: 700;
  }

  .th-dim {
    color: rgba(244, 244, 246, 0.58);
    font-weight: 500;
  }

  .th-dl {
    color: #34d399;
    font-weight: 700;
  }

  .th-ul {
    color: #60a5fa;
    font-weight: 600;
  }

  .th-peers {
    color: rgba(244, 244, 246, 0.68);
  }

  .th-sep {
    color: rgba(255, 255, 255, 0.22);
  }

  .skip-btn-wrap {
    position: fixed;
    right: 34px;
    bottom: 118px;
    z-index: 9300;
  }

  .skip-op-ed-btn {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    background: rgba(14, 14, 18, 0.92);
    backdrop-filter: blur(16px);
    border: 1px solid rgba(249, 115, 22, 0.65);
    color: #fff;
    font-size: 13.5px;
    font-weight: 700;
    padding: 10px 18px;
    border-radius: 10px;
    cursor: pointer;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.7);
    transition: all 140ms ease;
  }

  .skip-op-ed-btn:hover {
    background: #f97316;
    border-color: #f97316;
    transform: translateY(-1px);
  }

  .skip-key {
    font-size: 11px;
    padding: 2px 6px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.14);
    color: #ff9c85;
  }

  /* Interactive Subtitle Stage */
  .subtitle-stage {
    position: fixed;
    left: 50%;
    bottom: 98px;
    transform: translateX(-50%);
    z-index: 9400;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    max-width: 86vw;
    pointer-events: none;
  }

  .subtitle-stage.interactive {
    pointer-events: auto;
  }

  .subtitle-box {
    background: transparent;
    padding: 4px 12px;
    text-align: center;
    line-height: 1.45;
    font-size: clamp(23px, 2.6vw, 36px);
    font-weight: 700;
    color: #fff;
    paint-order: stroke fill;
    -webkit-text-stroke: 4.5px rgba(0, 0, 0, 0.96);
    text-shadow:
      0 2px 5px rgba(0, 0, 0, 0.95),
      0 0 12px rgba(0, 0, 0, 0.85);
  }

  .sub-char {
    display: inline-block;
    padding: 1px 1px;
    border-radius: 4px;
    transition: background 90ms ease, color 90ms ease;
  }

  .subtitle-stage.interactive .sub-char {
    cursor: pointer;
  }

  .subtitle-stage.interactive .sub-char:hover {
    background: rgba(249, 115, 22, 0.28);
  }

  .sub-char.tracked {
    border-bottom: 2.5px dotted rgba(244, 162, 97, 0.92);
  }

  .sub-char.highlighted {
    background: rgba(249, 115, 22, 0.42);
    color: #ffe0d6;
    border-bottom: 2px solid #f97316;
  }

  /* Bottom Bar */
  .player-bottom-bar {
    padding: 28px 28px 20px;
    background: linear-gradient(0deg, rgba(8, 8, 12, 0.94) 0%, rgba(8, 8, 12, 0.55) 65%, transparent 100%);
    display: flex;
    flex-direction: column;
    gap: 12px;
    opacity: 0;
    transform: translateY(6px);
    transition: opacity 180ms ease, transform 180ms ease;
    pointer-events: none;
  }

  .player-bottom-bar.visible {
    opacity: 1;
    transform: translateY(0);
    pointer-events: auto;
  }

  .timeline-row {
    position: relative;
    width: 100%;
    padding: 6px 0;
  }

  .timeline-tooltip {
    position: absolute;
    bottom: 22px;
    transform: translateX(-50%);
    background: rgba(14, 14, 18, 0.94);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 6px;
    padding: 4px 8px;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 600;
    color: #f4f4f6;
    white-space: nowrap;
    pointer-events: none;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.65);
  }

  .tt-time {
    font-variant-numeric: tabular-nums;
    color: #ff7a59;
  }

  .tt-chapter {
    color: rgba(244, 244, 246, 0.78);
    border-left: 1px solid rgba(255, 255, 255, 0.16);
    padding-left: 6px;
  }

  .scrubber-track {
    position: relative;
    height: 5px;
    background: rgba(255, 255, 255, 0.18);
    border-radius: 99px;
    overflow: visible;
    transition: height 120ms ease;
  }

  .timeline-row:hover .scrubber-track {
    height: 7px;
  }

  .scrubber-fill {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    background: linear-gradient(90deg, #f97316, #ff8566);
    border-radius: 99px;
    pointer-events: none;
  }

  .scrubber-thumb {
    position: absolute;
    right: -6px;
    top: 50%;
    transform: translateY(-50%) scale(0);
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 0 0 3px rgba(249, 115, 22, 0.55);
    transition: transform 120ms ease;
  }

  .timeline-row:hover .scrubber-thumb {
    transform: translateY(-50%) scale(1);
  }

  .segment-marker {
    position: absolute;
    top: 0;
    bottom: 0;
    border-radius: 99px;
    pointer-events: none;
  }

  .segment-marker.op {
    background: rgba(56, 189, 248, 0.65);
  }

  .segment-marker.ed {
    background: rgba(251, 191, 36, 0.65);
  }

  .chapter-tick {
    position: absolute;
    top: -1px;
    bottom: -1px;
    width: 2px;
    background: rgba(255, 255, 255, 0.65);
    pointer-events: none;
  }

  .scrubber-input {
    position: absolute;
    inset: -8px 0;
    width: 100%;
    height: 22px;
    opacity: 0;
    cursor: pointer;
    margin: 0;
  }

  .controls-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }

  .controls-left,
  .controls-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .ctrl-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    background: rgba(255, 255, 255, 0.07);
    border: 1px solid rgba(255, 255, 255, 0.1);
    color: #f4f4f6;
    font-size: 12px;
    font-weight: 600;
    height: 34px;
    padding: 0 12px;
    border-radius: 8px;
    cursor: pointer;
    transition: all 120ms ease;
  }

  .ctrl-btn.icon-only {
    padding: 0 9px;
  }

  .ctrl-btn:hover,
  .ctrl-btn.active {
    background: rgba(249, 115, 22, 0.2);
    border-color: rgba(249, 115, 22, 0.55);
    color: #fff;
  }

  .ctrl-btn.primary {
    background: #f97316;
    border-color: #f97316;
    color: #fff;
    padding: 0 14px;
  }

  .ctrl-btn.primary:hover {
    background: #ff6b4d;
    border-color: #ff6b4d;
  }

  .volume-group {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .vol-slider {
    width: 72px;
    accent-color: #f97316;
    cursor: pointer;
  }

  .time-readout {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12.5px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: #f4f4f6;
    margin-left: 6px;
  }

  .time-readout .sep,
  .time-readout .dur {
    color: rgba(244, 244, 246, 0.52);
  }

  .sub-delay-pill {
    display: inline-flex;
    align-items: center;
    height: 34px;
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 8px;
    overflow: hidden;
    font-size: 11.5px;
    font-weight: 600;
    color: rgba(244, 244, 246, 0.85);
  }

  .sub-delay-pill button {
    background: transparent;
    border: none;
    color: #ff7a59;
    font-weight: 700;
    height: 100%;
    padding: 0 9px;
    cursor: pointer;
  }

  .sub-delay-pill button:hover {
    background: rgba(255, 255, 255, 0.09);
  }

  .sub-delay-pill span {
    padding: 0 6px;
    font-variant-numeric: tabular-nums;
  }

  .menu-anchor {
    position: relative;
  }

  .popover-menu {
    position: absolute;
    right: 0;
    bottom: calc(100% + 10px);
    background: rgba(14, 14, 19, 0.96);
    backdrop-filter: blur(20px);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 10px;
    padding: 8px;
    min-width: 230px;
    max-height: 300px;
    overflow-y: auto;
    box-shadow: 0 18px 44px rgba(0, 0, 0, 0.8);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .speed-menu {
    min-width: 110px;
  }

  .menu-section {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .menu-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: rgba(244, 244, 246, 0.48);
    padding: 4px 8px;
  }

  .mini-action {
    background: rgba(249, 115, 22, 0.16);
    border: 1px solid rgba(249, 115, 22, 0.4);
    color: #ff8566;
    font-size: 10px;
    font-weight: 600;
    padding: 2px 7px;
    border-radius: 4px;
    cursor: pointer;
  }

  .menu-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    background: transparent;
    border: none;
    color: #f4f4f6;
    font-size: 12px;
    padding: 7px 10px;
    border-radius: 6px;
    text-align: left;
    cursor: pointer;
  }

  .menu-item:hover {
    background: rgba(255, 255, 255, 0.08);
  }

  .menu-item.selected {
    background: rgba(249, 115, 22, 0.22);
    color: #ff8566;
    font-weight: 600;
  }

  .lang-tag {
    font-size: 10px;
    text-transform: uppercase;
    padding: 1px 5px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.1);
    color: rgba(244, 244, 246, 0.72);
  }
</style>
