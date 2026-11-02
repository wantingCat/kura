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

  // Yomitan Memento lookup state (only active while paused)
  let lookupResult = $state<LookupResult | null>(null);
  let highlightStart = $state<number>(-1);
  let highlightEnd = $state<number>(-1);
  let popupAnchorX = $state<number>(640);
  let popupAnchorY = $state<number>(560);
  let popupPinned = $state<boolean>(false);
  let lookupReqCounter = 0;

  // In-window HTML5 video surface binding + automatic live mpv transcoding fallback
  let html5Video = $state<HTMLVideoElement | null>(null);
  let useTranscode = $state(false);
  let transcodeVideoCodec = $state<"copy" | "h264">("copy");
  let transcodeStartOffset = $state(0);
  let transcodeRev = $state(0);
  let initialSeekDone = $state(false);
  let lastLoadedPath = $state("");

  // Active torrent session HUD stats (downloaded %, MB downloaded/total, ↓ speed, ↑ speed)
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

  $effect(() => {
    const curPath = `${playerState.anilistId}:${playerState.epKey}:${playerState.path}`;
    if (curPath !== lastLoadedPath) {
      lastLoadedPath = curPath;
      useTranscode = false;
      transcodeVideoCodec = "copy";
      transcodeStartOffset = playerState.position > 1.0 ? playerState.position : 0;
      transcodeRev = 0;
      initialSeekDone = false;
      clearLookup();
    }
  });

  // Automatically engage zero-CPU live mpv audio remux (`vc=copy`) for local files whose
  // Matroska audio codec is unsupported by WebView2 (FLAC, E-AC3, AC3, DTS, TrueHD, PCM)
  $effect(() => {
    const isRemoteStream =
      playerState.path.startsWith("http://") || playerState.path.startsWith("https://");
    if (
      !isRemoteStream &&
      !useTranscode &&
      playerState.transcodeBaseUrl &&
      playerState.audioTracks.length > 0
    ) {
      const selectedTrack =
        playerState.audioTracks.find((t) => t.selected) ?? playerState.audioTracks[0];
      const codec = (selectedTrack?.codec ?? "").toLowerCase();
      const needsAudioTranscode =
        (selectedTrack && selectedTrack.id > 1) ||
        /flac|eac3|ac3|dts|truehd|pcm|mlp/.test(codec);
      if (needsAudioTranscode) {
        transcodeVideoCodec = "copy";
        transcodeStartOffset = playerState.position > 1.0 ? playerState.position : 0;
        useTranscode = true;
      }
    }
  });

  let activeVideoSrc = $derived.by(() => {
    if (useTranscode && playerState.transcodeBaseUrl) {
      const aid = playerState.aid ?? 0;
      return `${playerState.transcodeBaseUrl}?start=${transcodeStartOffset.toFixed(1)}&aid=${aid}&vc=${transcodeVideoCodec}&rev=${transcodeRev}`;
    }
    if (playerState.streamUrl) {
      return playerState.streamUrl;
    }
    if (playerState.path.startsWith("http://") || playerState.path.startsWith("https://")) {
      return playerState.path;
    }
    return "";
  });

  // Split subtitle text into an array of Unicode codepoints with their character indices
  let subChars = $derived.by(() => {
    const raw = playerState.subText || "";
    return Array.from(raw).map((ch, idx) => ({ ch, idx }));
  });

  // Detect active OP / ED skip window
  let activeSkip = $derived.by(() => {
    const pos = playerState.position;
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

  // Automatically clear Yomitan popup when video resumes playing (Memento behavior)
  $effect(() => {
    if (!playerState.paused) {
      clearLookup();
    }
  });

  // Sync Win32 video bounds on window resize
  $effect(() => {
    if (!playerState.active) return;
    const onResize = () => {
      void api.builtinPlayerCommand({ type: "sync_bounds" });
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  });

  function clearLookup() {
    lookupResult = null;
    highlightStart = -1;
    highlightEnd = -1;
    popupPinned = false;
  }

  function bumpControls() {
    controlsVisible = true;
    if (hideTimer) clearTimeout(hideTimer);
    if (!playerState.paused) {
      hideTimer = setTimeout(() => {
        if (!playerState.paused && activeMenu === "none") {
          controlsVisible = false;
        }
      }, 2800);
    }
  }

  async function triggerLookup(charIdx: number, targetEl: HTMLElement, pin: boolean) {
    // Memento rule: user MUST pause first before hovering triggers Yomitan
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
        if (pin) {
          popupPinned = true;
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

  async function seekTo(sec: number) {
    const clamped = Math.max(0, Math.min(sec, playerState.duration || sec));
    if (useTranscode && playerState.transcodeBaseUrl) {
      transcodeStartOffset = clamped;
      transcodeRev += 1;
    } else if (html5Video) {
      html5Video.currentTime = clamped;
    }
    await api.builtinPlayerCommand({ type: "seek", position: clamped });
  }

  async function seekRelative(delta: number) {
    await seekTo(playerState.position + delta);
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
    await api.builtinPlayerCommand({ type: "set_audio_track", id });
    if (playerState.transcodeBaseUrl && playerState.audioTracks.length > 1) {
      transcodeStartOffset = playerState.position;
      transcodeVideoCodec = "copy";
      useTranscode = true;
      transcodeRev += 1;
    }
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
        void setVolume(Math.min(130, playerState.volume + 5));
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
    class="memento-player-root"
    class:controls-hidden={!controlsVisible && !playerState.paused}
    onmousemove={bumpControls}
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
        onloadedmetadata={(e) => {
          const v = e.currentTarget;
          if (!useTranscode && v.videoWidth === 0 && playerState.transcodeBaseUrl) {
            transcodeVideoCodec = "h264";
            transcodeStartOffset = playerState.position > 1.0 ? playerState.position : 0;
            useTranscode = true;
            return;
          }
          if (!useTranscode && !initialSeekDone && playerState.position > 1.0) {
            initialSeekDone = true;
            v.currentTime = playerState.position;
          }
          v.volume = Math.max(0, Math.min(1, playerState.volume / 100));
          v.muted = playerState.muted;
          v.playbackRate = playerState.speed;
        }}
        onerror={() => {
          if (!useTranscode && playerState.transcodeBaseUrl) {
            transcodeVideoCodec = "h264";
            transcodeStartOffset = playerState.position > 1.0 ? playerState.position : 0;
            useTranscode = true;
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
            v.currentTime > 2.0
          ) {
            const audioBytes = (v as unknown as { webkitAudioDecodedByteCount?: number }).webkitAudioDecodedByteCount;
            if (typeof audioBytes === "number" && audioBytes === 0) {
              transcodeVideoCodec = "copy";
              transcodeStartOffset = v.currentTime > 1.0 ? v.currentTime : 0;
              useTranscode = true;
              return;
            }
          }
          const pos = useTranscode ? transcodeStartOffset + v.currentTime : v.currentTime;
          const dur = Number.isFinite(v.duration) && v.duration > 0 ? v.duration : playerState.duration;
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
          const dur = Number.isFinite(v.duration) && v.duration > 0 ? v.duration : playerState.duration;
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
          const dur = Number.isFinite(v.duration) && v.duration > 0 ? v.duration : playerState.duration;
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

    <!-- Top Bar -->
    <header
      class="player-top-bar"
      class:visible={controlsVisible || playerState.paused}
      onclick={(e) => e.stopPropagation()}
    >
      <button type="button" class="back-btn" onclick={closePlayer} title="Exit Player (Esc)">
        ← Back to Kura
      </button>
      <div class="title-stack">
        <div class="show-title">{playerState.showTitle}</div>
        <div class="ep-title">{playerState.episodeTitle}</div>
      </div>
      <div class="top-right-badges">
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
            <span class="th-peers">{activeTorrentSession.peers} peers</span>
          </div>
        {/if}
        <span class="memento-pill" class:ready={playerState.paused}>
          {playerState.paused ? "● Memento Hover Active" : "Pause to Hover Yomitan"}
        </span>
        <button
          type="button"
          class="back-btn"
          onclick={() => api.builtinPlayerCommand({ type: "open_external_mpv" })}
          title="Pop out into standalone external mpv window"
        >
          mpv ↗
        </button>
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

    <!-- Memento Interactive Subtitle Layer -->
    {#if subChars.length > 0}
      <div
        class="subtitle-stage"
        class:interactive={playerState.paused}
        onmouseleave={handleSubtitleMouseLeave}
        onclick={(e) => e.stopPropagation()}
      >
        {#if playerState.paused}
          <div class="memento-hint-pill">Hover any Japanese word for Yomitan · Click to pin</div>
        {/if}
        <div class="subtitle-box">
          {#each subChars as item (item.idx)}
            {#if item.ch === "\n"}
              <br />
            {:else}
              <span
                class="sub-char"
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
        onClose={clearLookup}
      />
    {/if}

    <!-- Bottom Controls & Scrubber -->
    <footer
      class="player-bottom-bar"
      class:visible={controlsVisible || playerState.paused}
      onclick={(e) => e.stopPropagation()}
    >
      <!-- Timeline Scrubber with OP/ED & Chapter Markers -->
      <div class="timeline-row">
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
              style="width: {Math.min(100, (playerState.position / playerState.duration) * 100)}%;"
            ></div>
          {/if}
          <input
            type="range"
            class="scrubber-input"
            min="0"
            max={playerState.duration > 0 ? playerState.duration : 100}
            step="0.2"
            value={playerState.position}
            oninput={(e) => void seekTo(Number((e.currentTarget as HTMLInputElement).value))}
          />
        </div>
      </div>

      <div class="controls-row">
        <div class="controls-left">
          <button type="button" class="ctrl-btn primary" onclick={togglePlayPause} title="Play / Pause (Space)">
            {playerState.paused ? "▶" : "⏸"}
          </button>
          <button type="button" class="ctrl-btn" onclick={() => seekRelative(-5)} title="Rewind 5s (←)">
            ⟲ 5s
          </button>
          <button type="button" class="ctrl-btn" onclick={() => seekRelative(5)} title="Forward 5s (→)">
            5s ⟳
          </button>

          {#if playerState.hasNextEpisode}
            <button
              type="button"
              class="ctrl-btn"
              onclick={() => api.builtinPlayerCommand({ type: "next_episode" })}
              title="Next Episode"
            >
              ⏭ Next
            </button>
          {/if}

          <div class="volume-group">
            <button type="button" class="ctrl-btn" onclick={toggleMute} title="Mute (M)">
              {playerState.muted || playerState.volume === 0 ? "🔇" : "🔊"}
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
            <span>{formatTime(playerState.position)}</span>
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
              💬 Subtitles & Audio
            </button>
            {#if activeMenu === "tracks"}
              <div class="popover-menu tracks-menu">
                <div class="menu-section">
                  <div class="menu-heading">
                    <span>Subtitles</span>
                    <button type="button" class="mini-action" onclick={pickExternalSubtitle}>
                      + Load .srt/.ass
                    </button>
                  </div>
                  <button
                    type="button"
                    class="menu-item"
                    class:selected={playerState.sid === null}
                    onclick={() => api.builtinPlayerCommand({ type: "set_sub_track", id: 0 })}
                  >
                    Off
                  </button>
                  {#each playerState.subTracks as tr}
                    <button
                      type="button"
                      class="menu-item"
                      class:selected={tr.selected}
                      onclick={() => api.builtinPlayerCommand({ type: "set_sub_track", id: tr.id })}
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
              {playerState.speed}x
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
                    {sp}x
                  </button>
                {/each}
              </div>
            {/if}
          </div>

          <button type="button" class="ctrl-btn" onclick={toggleFullscreen} title="Fullscreen (F)">
            ⛶
          </button>
        </div>
      </div>
    </footer>
  </div>
{/if}

<style>
  .memento-player-root {
    position: fixed;
    inset: 0;
    z-index: 9000;
    background: #000;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    user-select: none;
  }

  .memento-player-root.controls-hidden {
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

  .player-top-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 18px 26px 28px;
    background: linear-gradient(to bottom, rgba(12, 8, 9, 0.88), rgba(12, 8, 9, 0));
    opacity: 0;
    transform: translateY(-8px);
    transition: opacity 180ms ease, transform 180ms ease;
    pointer-events: none;
  }

  .player-top-bar.visible {
    opacity: 1;
    transform: translateY(0);
    pointer-events: auto;
  }

  .back-btn {
    background: rgba(20, 12, 13, 0.78);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.14);
    color: #f7ede2;
    font-size: 13px;
    font-weight: 600;
    padding: 8px 14px;
    border-radius: 8px;
    cursor: pointer;
    transition: all 140ms ease;
  }

  .back-btn:hover {
    background: rgba(231, 111, 81, 0.28);
    border-color: rgba(231, 111, 81, 0.6);
  }

  .title-stack {
    flex: 1;
    min-width: 0;
    text-align: center;
  }

  .show-title {
    font-size: 15px;
    font-weight: 700;
    color: #fff;
    text-shadow: 0 2px 8px rgba(0, 0, 0, 0.85);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ep-title {
    font-size: 12px;
    color: rgba(247, 237, 226, 0.75);
    text-shadow: 0 1px 6px rgba(0, 0, 0, 0.85);
  }

  .memento-pill {
    font-size: 11px;
    font-weight: 600;
    padding: 6px 12px;
    border-radius: 99px;
    background: rgba(20, 12, 13, 0.75);
    border: 1px solid rgba(255, 255, 255, 0.14);
    color: rgba(247, 237, 226, 0.75);
  }

  .memento-pill.ready {
    background: rgba(244, 162, 97, 0.2);
    border-color: rgba(244, 162, 97, 0.65);
    color: #f4a261;
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
    background: rgba(16, 11, 12, 0.84);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.16);
    color: #f7ede2;
    white-space: nowrap;
  }

  .th-prog {
    color: #f4a261;
    font-weight: 700;
  }

  .th-dim {
    color: rgba(247, 237, 226, 0.68);
    font-weight: 500;
  }

  .th-dl {
    color: #52b788;
    font-weight: 700;
  }

  .th-ul {
    color: #74c0fc;
    font-weight: 600;
  }

  .th-peers {
    color: rgba(247, 237, 226, 0.72);
  }

  .th-sep {
    color: rgba(255, 255, 255, 0.25);
  }

  .skip-btn-wrap {
    position: fixed;
    right: 34px;
    bottom: 120px;
    z-index: 9300;
  }

  .skip-op-ed-btn {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    background: rgba(20, 12, 13, 0.9);
    backdrop-filter: blur(12px);
    border: 1.5px solid #f4a261;
    color: #fff;
    font-size: 14px;
    font-weight: 700;
    padding: 10px 18px;
    border-radius: 10px;
    cursor: pointer;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.65);
    transition: all 140ms ease;
  }

  .skip-op-ed-btn:hover {
    background: #e76f51;
    border-color: #e76f51;
    transform: translateY(-1px);
  }

  .skip-key {
    font-size: 11px;
    padding: 2px 6px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.16);
    color: #f4a261;
  }

  /* Memento Interactive Subtitle Stage */
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

  .memento-hint-pill {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.03em;
    color: #f4a261;
    background: rgba(20, 12, 13, 0.86);
    border: 1px solid rgba(244, 162, 97, 0.4);
    padding: 3px 10px;
    border-radius: 99px;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.5);
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

  .subtitle-stage.interactive .subtitle-box {
    background: transparent;
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
    background: rgba(244, 162, 97, 0.25);
  }

  .sub-char.highlighted {
    background: rgba(231, 111, 81, 0.42);
    color: #ffd8be;
    border-bottom: 2px solid #f4a261;
  }

  /* Bottom Bar */
  .player-bottom-bar {
    padding: 24px 26px 18px;
    background: linear-gradient(to top, rgba(12, 8, 9, 0.92), rgba(12, 8, 9, 0));
    display: flex;
    flex-direction: column;
    gap: 10px;
    opacity: 0;
    transform: translateY(8px);
    transition: opacity 180ms ease, transform 180ms ease;
    pointer-events: none;
  }

  .player-bottom-bar.visible {
    opacity: 1;
    transform: translateY(0);
    pointer-events: auto;
  }

  .timeline-row {
    width: 100%;
    padding: 4px 0;
  }

  .scrubber-track {
    position: relative;
    height: 7px;
    background: rgba(255, 255, 255, 0.2);
    border-radius: 99px;
    overflow: visible;
  }

  .scrubber-fill {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    background: linear-gradient(90deg, #e76f51, #f4a261);
    border-radius: 99px;
    pointer-events: none;
  }

  .segment-marker {
    position: absolute;
    top: 0;
    bottom: 0;
    border-radius: 99px;
    pointer-events: none;
  }

  .segment-marker.op {
    background: rgba(42, 157, 143, 0.75);
  }

  .segment-marker.ed {
    background: rgba(233, 196, 106, 0.75);
  }

  .chapter-tick {
    position: absolute;
    top: -2px;
    bottom: -2px;
    width: 2px;
    background: rgba(255, 255, 255, 0.7);
    pointer-events: none;
  }

  .scrubber-input {
    position: absolute;
    inset: -6px 0;
    width: 100%;
    height: 20px;
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
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.12);
    color: #f7ede2;
    font-size: 12px;
    font-weight: 600;
    padding: 6px 11px;
    border-radius: 8px;
    cursor: pointer;
    transition: all 120ms ease;
  }

  .ctrl-btn:hover,
  .ctrl-btn.active {
    background: rgba(244, 162, 97, 0.24);
    border-color: rgba(244, 162, 97, 0.6);
    color: #fff;
  }

  .ctrl-btn.primary {
    background: #e76f51;
    border-color: #e76f51;
    color: #fff;
    font-size: 14px;
    padding: 6px 14px;
  }

  .volume-group {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .vol-slider {
    width: 72px;
    accent-color: #f4a261;
    cursor: pointer;
  }

  .time-readout {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: #f7ede2;
    margin-left: 6px;
  }

  .time-readout .sep,
  .time-readout .dur {
    color: rgba(247, 237, 226, 0.6);
  }

  .sub-delay-pill {
    display: inline-flex;
    align-items: center;
    background: rgba(255, 255, 255, 0.07);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 8px;
    overflow: hidden;
    font-size: 11px;
    color: rgba(247, 237, 226, 0.85);
  }

  .sub-delay-pill button {
    background: transparent;
    border: none;
    color: #f4a261;
    font-weight: 700;
    padding: 5px 8px;
    cursor: pointer;
  }

  .sub-delay-pill button:hover {
    background: rgba(255, 255, 255, 0.1);
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
    background: rgba(20, 12, 13, 0.96);
    backdrop-filter: blur(16px);
    border: 1px solid rgba(244, 162, 97, 0.35);
    border-radius: 10px;
    padding: 8px;
    min-width: 220px;
    max-height: 280px;
    overflow-y: auto;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.75);
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
    text-transform: uppercase;
    color: rgba(247, 237, 226, 0.55);
    padding: 4px 8px;
  }

  .mini-action {
    background: rgba(244, 162, 97, 0.18);
    border: 1px solid rgba(244, 162, 97, 0.4);
    color: #f4a261;
    font-size: 10px;
    font-weight: 600;
    padding: 2px 6px;
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
    color: #f7ede2;
    font-size: 12px;
    padding: 6px 10px;
    border-radius: 6px;
    text-align: left;
    cursor: pointer;
  }

  .menu-item:hover {
    background: rgba(255, 255, 255, 0.08);
  }

  .menu-item.selected {
    background: rgba(231, 111, 81, 0.25);
    color: #f4a261;
    font-weight: 600;
  }

  .lang-tag {
    font-size: 10px;
    text-transform: uppercase;
    padding: 1px 5px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.1);
    color: rgba(247, 237, 226, 0.7);
  }
</style>

