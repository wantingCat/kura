<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    api,
    img,
    type ImmersionOverview,
    type ImmersionWordItem,
  } from "$lib/api";
  import { app } from "$lib/app.svelte";
  import Icon from "$lib/components/Icon.svelte";

  let overview = $state<ImmersionOverview | null>(null);
  let loading = $state(true);

  // Word bank filter & sort state
  let statusFilter = $state<"all" | "learning" | "known" | "ignored">("all");
  let sortBy = $state<"count" | "recent" | "oldest">("count");
  let searchQuery = $state("");
  let flashcardMode = $state(false);
  let revealedIds = $state<Set<number>>(new Set());
  let selectedWordId = $state<number | null>(null);

  // Chart range
  let chartDays = $state<14 | 30>(14);

  // Modals: Daily Goal & Manual Log
  let goalModalOpen = $state(false);
  let goalInputMinutes = $state(60);
  let logModalOpen = $state(false);
  let logMinutes = $state(24);
  let logLabel = $state("");
  let logAnilistId = $state<number | null>(null);
  let submittingLog = $state(false);

  onMount(async () => {
    await loadOverview();
  });

  async function loadOverview() {
    loading = true;
    try {
      const data = await api.getImmersionOverview();
      overview = data;
      goalInputMinutes = data.dailyGoalMinutes || 60;
      if (selectedWordId === null && data.words.length > 0) {
        const firstActive = data.words.find((w) => w.status !== "ignored") ?? data.words[0];
        selectedWordId = firstActive?.id ?? null;
      }
    } catch (e) {
      app.toast(`Failed to load immersion data: ${e}`, "error");
    } finally {
      loading = false;
    }
  }

  let filteredWords = $derived.by(() => {
    if (!overview) return [];
    const q = searchQuery.trim().toLowerCase();
    let list = overview.words.filter((w) => {
      if (statusFilter === "all") {
        return w.status !== "ignored";
      }
      return w.status === statusFilter;
    });

    if (q) {
      list = list.filter((w) => {
        if (w.expression.toLowerCase().includes(q)) return true;
        if (w.reading.toLowerCase().includes(q)) return true;
        if (w.matchedForms.some((f) => f.toLowerCase().includes(q))) return true;
        if (w.glossaries.some((g) => g.toLowerCase().includes(q))) return true;
        if (w.contexts.some((c) => c.sentence.toLowerCase().includes(q))) return true;
        return false;
      });
    }

    const sorted = [...list];
    if (sortBy === "count") {
      sorted.sort((a, b) => b.lookupCount - a.lookupCount || b.lastSeenAt - a.lastSeenAt);
    } else if (sortBy === "recent") {
      sorted.sort((a, b) => b.lastSeenAt - a.lastSeenAt || b.lookupCount - a.lookupCount);
    } else {
      sorted.sort((a, b) => a.firstSeenAt - b.firstSeenAt);
    }
    return sorted;
  });

  let selectedWord = $derived.by(() => {
    if (!overview || selectedWordId === null) return filteredWords[0] ?? null;
    return (
      overview.words.find((w) => w.id === selectedWordId) ??
      filteredWords[0] ??
      null
    );
  });

  let chartPoints = $derived.by(() => {
    if (!overview) return [];
    const pts = overview.dailyActivity;
    return chartDays === 14 ? pts.slice(-14) : pts;
  });

  let maxChartSeconds = $derived.by(() => {
    const goalSec = (overview?.dailyGoalMinutes ?? 60) * 60;
    const peak = chartPoints.reduce((m, p) => Math.max(m, p.activeSeconds), 0);
    return Math.max(goalSec, peak, 1800);
  });

  let goalProgressPct = $derived.by(() => {
    if (!overview) return 0;
    const goalSec = Math.max(300, overview.dailyGoalMinutes * 60);
    return Math.min(100, Math.round((overview.todayImmersionSec / goalSec) * 100));
  });

  function formatDuration(seconds: number): string {
    const sec = Math.max(0, Math.floor(seconds));
    const hrs = Math.floor(sec / 3600);
    const mins = Math.floor((sec % 3600) / 60);
    if (hrs > 0) {
      return `${hrs}h ${mins}m`;
    }
    if (mins > 0) {
      return `${mins}m`;
    }
    return sec > 0 ? `${sec}s` : "0m";
  }

  function formatHoursDecimal(seconds: number): string {
    const hrs = Math.max(0, seconds) / 3600;
    return hrs >= 10 ? hrs.toFixed(1) : hrs.toFixed(1);
  }

  function formatTimestamp(sec: number | null): string {
    if (sec === null || !Number.isFinite(sec) || sec < 0) return "";
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  function formatRelativeTime(unixSec: number): string {
    const diff = Math.max(0, Math.floor(Date.now() / 1000) - unixSec);
    if (diff < 60) return "just now";
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    const days = Math.floor(diff / 86400);
    return `${days}d ago`;
  }

  function splitMoras(reading: string): string[] {
    const small = new Set(["ゃ", "ゅ", "ょ", "ぁ", "ぃ", "ぅ", "ぇ", "ぉ", "ャ", "ュ", "ョ", "ァ", "ィ", "ゥ", "ェ", "ォ"]);
    const chars = Array.from(reading);
    const out: string[] = [];
    for (let i = 0; i < chars.length; i++) {
      if (i + 1 < chars.length && small.has(chars[i + 1])) {
        out.push(chars[i] + chars[i + 1]);
        i++;
      } else {
        out.push(chars[i]);
      }
    }
    return out;
  }

  function isMoraHigh(moraIndex: number, downstep: number): boolean {
    if (downstep === 0) return moraIndex > 0;
    if (downstep === 1) return moraIndex === 0;
    return moraIndex > 0 && moraIndex < downstep;
  }

  function speakJapanese(text: string, e?: MouseEvent) {
    e?.stopPropagation();
    if (typeof window === "undefined" || !("speechSynthesis" in window)) return;
    try {
      window.speechSynthesis.cancel();
      const u = new SpeechSynthesisUtterance(text);
      u.lang = "ja-JP";
      u.rate = 0.95;
      window.speechSynthesis.speak(u);
    } catch {
      // ignore
    }
  }

  function toggleReveal(id: number, e?: MouseEvent) {
    e?.stopPropagation();
    const next = new Set(revealedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    revealedIds = next;
  }

  async function updateWordStatus(
    word: ImmersionWordItem,
    nextStatus: "learning" | "known" | "ignored",
    e?: MouseEvent
  ) {
    e?.stopPropagation();
    if (!overview) return;
    const prevStatus = word.status;
    if (prevStatus === nextStatus) return;

    // Optimistic update
    overview.words = overview.words.map((w) =>
      w.id === word.id ? { ...w, status: nextStatus } : w
    );
    recalcCounts();

    try {
      await api.setWordStatus({
        expression: word.expression,
        reading: word.reading,
        status: nextStatus,
      });
    } catch (err) {
      app.toast(`Couldn't update word status: ${err}`, "error");
      await loadOverview();
    }
  }

  async function removeWord(word: ImmersionWordItem, e?: MouseEvent) {
    e?.stopPropagation();
    if (!overview) return;
    overview.words = overview.words.filter((w) => w.id !== word.id);
    if (selectedWordId === word.id) {
      selectedWordId = filteredWords[0]?.id ?? null;
    }
    recalcCounts();
    try {
      await api.deleteImmersionWord(word.id);
    } catch (err) {
      app.toast(`Couldn't remove word: ${err}`, "error");
      await loadOverview();
    }
  }

  function recalcCounts() {
    if (!overview) return;
    let learning = 0;
    let known = 0;
    let ignored = 0;
    let lookups = 0;
    for (const w of overview.words) {
      if (w.status === "known") {
        known++;
        lookups += w.lookupCount;
      } else if (w.status === "ignored") {
        ignored++;
      } else {
        learning++;
        lookups += w.lookupCount;
      }
    }
    overview.learningWordsCount = learning;
    overview.knownWordsCount = known;
    overview.ignoredWordsCount = ignored;
    overview.uniqueWordsCount = learning + known;
    overview.totalLookups = lookups;
  }

  async function saveDailyGoal() {
    try {
      const saved = await api.setImmersionDailyGoal(goalInputMinutes);
      if (overview) overview.dailyGoalMinutes = saved;
      goalModalOpen = false;
      app.toast(`Daily immersion goal set to ${saved} minutes`, "info");
    } catch (e) {
      app.toast(`Failed to save goal: ${e}`, "error");
    }
  }

  async function submitManualLog() {
    if (submittingLog) return;
    submittingLog = true;
    try {
      const selectedCard = logAnilistId
        ? app.cards.find((c) => c.anilistId === logAnilistId)
        : null;
      const effectiveLabel =
        logLabel.trim() ||
        selectedCard?.titleEnglish ||
        selectedCard?.titleRomaji ||
        "Japanese Immersion";
      await api.logManualImmersion({
        minutes: logMinutes,
        label: effectiveLabel,
        anilistId: logAnilistId,
      });
      logModalOpen = false;
      logLabel = "";
      app.toast(`Logged ${logMinutes}m of immersion time`, "info");
      await loadOverview();
    } catch (e) {
      app.toast(`Failed to log immersion time: ${e}`, "error");
    } finally {
      submittingLog = false;
    }
  }

  function highlightTargetInSentence(
    sentence: string,
    matchedSurface: string,
    expression: string
  ): { before: string; match: string; after: string } | null {
    const target =
      matchedSurface && sentence.includes(matchedSurface)
        ? matchedSurface
        : sentence.includes(expression)
          ? expression
          : "";
    if (!target) return null;
    const idx = sentence.indexOf(target);
    if (idx < 0) return null;
    return {
      before: sentence.slice(0, idx),
      match: target,
      after: sentence.slice(idx + target.length),
    };
  }
</script>

<div class="immersion-page">
  <!-- Header -->
  <header class="im-header">
    <div class="im-title-group">
      <div class="im-eyebrow">
        <Icon name="book-open" size={15} />
        <span>JAPANESE IMMERSION SUITE</span>
      </div>
      <h1>Immersion & Vocabulary Bank</h1>
      <p class="im-subtitle">
        Track your active anime immersion hours and master the words you look up most frequently with Yomitan.
      </p>
    </div>

    <div class="im-header-actions">
      <button
        type="button"
        class="im-btn"
        class:active={flashcardMode}
        onclick={() => {
          flashcardMode = !flashcardMode;
          revealedIds = new Set();
        }}
        title="Hide English meanings until clicked so you can self-test your top looked-up words"
      >
        <Icon name={flashcardMode ? "eye-off" : "eye"} size={15} />
        <span>{flashcardMode ? "Self-Test Mode: ON" : "Self-Test Mode"}</span>
      </button>

      <button
        type="button"
        class="im-btn"
        onclick={() => (goalModalOpen = true)}
        title="Set your daily immersion target"
      >
        <Icon name="clock" size={15} />
        <span>Goal: {overview?.dailyGoalMinutes ?? 60}m/day</span>
      </button>

      <button
        type="button"
        class="im-btn primary"
        onclick={() => (logModalOpen = true)}
      >
        <Icon name="plus" size={15} />
        <span>Log Immersion Time</span>
      </button>
    </div>
  </header>

  {#if loading && !overview}
    <div class="im-loading">
      <span class="spinner"></span>
      <span>Loading immersion analytics & vocabulary bank...</span>
    </div>
  {:else if overview}
    <!-- Top Immersion Time & Vocabulary Metrics -->
    <section class="im-metrics-grid">
      <!-- Card 1: Today's Immersion + Goal Ring -->
      <div class="im-metric-card goal-card">
        <div class="goal-ring-wrap">
          <svg class="goal-ring" viewBox="0 0 80 80">
            <circle class="ring-bg" cx="40" cy="40" r="33" />
            <circle
              class="ring-fg"
              cx="40"
              cy="40"
              r="33"
              stroke-dasharray={2 * Math.PI * 33}
              stroke-dashoffset={2 * Math.PI * 33 * (1 - goalProgressPct / 100)}
            />
          </svg>
          <div class="ring-pct">{goalProgressPct}%</div>
        </div>
        <div class="metric-info">
          <span class="metric-label">TODAY'S IMMERSION</span>
          <div class="metric-val">{formatDuration(overview.todayImmersionSec)}</div>
          <div class="metric-sub">
            Daily target: <strong>{overview.dailyGoalMinutes}m</strong>
            <button type="button" class="inline-link" onclick={() => (goalModalOpen = true)}>Edit</button>
          </div>
        </div>
      </div>

      <!-- Card 2: Weekly & Monthly Immersion -->
      <div class="im-metric-card">
        <div class="metric-info">
          <span class="metric-label">IMMERSION VELOCITY</span>
          <div class="metric-val">{formatDuration(overview.weekImmersionSec)} <small>this week</small></div>
          <div class="metric-sub">
            Last 30 days: <strong>{formatDuration(overview.monthImmersionSec)}</strong>
          </div>
        </div>
      </div>

      <!-- Card 3: All-Time Immersion & Streak -->
      <div class="im-metric-card">
        <div class="metric-info">
          <span class="metric-label">ALL-TIME IMMERSION</span>
          <div class="metric-val">
            {formatHoursDecimal(overview.totalImmersionSec)} <small>hrs</small>
          </div>
          <div class="metric-sub">
            <span class="streak-pill" class:lit={overview.currentStreakDays > 0}>
              🔥 {overview.currentStreakDays}d streak
            </span>
            <span>· {overview.totalEpisodesCompleted} eps</span>
            <button
              type="button"
              class="inline-link"
              onclick={() => {
                try {
                  localStorage.setItem("kura.settings.category", "integrations");
                } catch {}
                goto("/settings");
              }}
              title="Choose which AniList custom/private list counts toward your Japanese Immersion hours"
            >
              {overview.anilistImmersionList
                ? `List: ${overview.anilistImmersionList}`
                : "Link AniList List"}
            </button>
          </div>
        </div>
      </div>

      <!-- Card 4: Vocabulary Bank Breakdown -->
      <div class="im-metric-card">
        <div class="metric-info">
          <span class="metric-label">VOCABULARY BANK</span>
          <div class="metric-val">
            {overview.totalLookups} <small>lookups</small>
          </div>
          <div class="metric-sub vocab-pills">
            <span class="v-dot learning">● {overview.learningWordsCount} Tracked</span>
            <span class="v-dot known">✓ {overview.knownWordsCount} Known</span>
          </div>
        </div>
      </div>
    </section>

    <!-- Activity Chart & Top Immersed Anime Row -->
    <section class="im-analytics-row">
      <!-- Daily Immersion Chart -->
      <div class="im-panel chart-panel">
        <div class="panel-head">
          <div>
            <h2>Daily Immersion Activity</h2>
            <p class="panel-sub">Active watch time & dictionary lookups per day</p>
          </div>
          <div class="seg-toggle">
            <button
              type="button"
              class:active={chartDays === 14}
              onclick={() => (chartDays = 14)}
            >14 Days</button>
            <button
              type="button"
              class:active={chartDays === 30}
              onclick={() => (chartDays = 30)}
            >30 Days</button>
          </div>
        </div>

        <div class="bar-chart-wrap">
          <!-- Goal dashed line -->
          {#if overview.dailyGoalMinutes * 60 <= maxChartSeconds}
            {@const goalBottomPct = Math.min(96, ((overview.dailyGoalMinutes * 60) / maxChartSeconds) * 100)}
            <div class="goal-line" style="bottom: {goalBottomPct}%;">
              <span>Goal ({overview.dailyGoalMinutes}m)</span>
            </div>
          {/if}

          <div class="bars-grid" class:dense={chartDays === 30}>
            {#each chartPoints as pt (pt.dayKey)}
              {@const hPct = pt.activeSeconds > 0 ? Math.max(5, Math.min(100, (pt.activeSeconds / maxChartSeconds) * 100)) : 0}
              {@const metGoal = pt.activeSeconds >= overview.dailyGoalMinutes * 60}
              <div
                class="bar-col"
                title={`${pt.label}: ${formatDuration(pt.activeSeconds)} immersed · ${pt.lookupCount} lookups`}
              >
                <div class="bar-track">
                  {#if pt.lookupCount > 0}
                    <span class="bar-lookup-pip">{pt.lookupCount}</span>
                  {/if}
                  <div
                    class="bar-fill"
                    class:met-goal={metGoal}
                    style="height: {hPct}%;"
                  ></div>
                </div>
                <span class="bar-date">{pt.label.split(" ")[1]}</span>
              </div>
            {/each}
          </div>
        </div>
      </div>

      <!-- Top Immersed Anime Breakdown -->
      <div class="im-panel anime-panel">
        <div class="panel-head">
          <div>
            <h2>Immersion by Anime</h2>
            <p class="panel-sub">Time spent & words looked up per series</p>
          </div>
        </div>

        {#if overview.topAnime.length === 0}
          <div class="panel-empty">
            Watch any episode in Kura or click <strong>Log Immersion Time</strong> to start tracking time per anime.
          </div>
        {:else}
          <div class="top-anime-list">
            {#each overview.topAnime.slice(0, 5) as show}
              {@const cover = img(show.coverPath, show.coverUrl)}
              <button
                type="button"
                class="top-anime-row"
                onclick={() => show.anilistId > 0 && goto(`/anime/${show.anilistId}`)}
              >
                <div class="ta-cover">
                  {#if cover}
                    <img src={cover} alt="" loading="lazy" />
                  {:else}
                    <div class="ta-fallback"><Icon name="film" size={14} /></div>
                  {/if}
                </div>
                <div class="ta-info">
                  <div class="ta-title">{show.title}</div>
                  <div class="ta-meta">
                    <span>{show.episodesWatched} ep{show.episodesWatched === 1 ? "" : "s"}</span>
                    {#if show.wordsLookedUp > 0}
                      <span>· {show.wordsLookedUp} word{show.wordsLookedUp === 1 ? "" : "s"} mined</span>
                    {/if}
                  </div>
                </div>
                <div class="ta-time">{formatDuration(show.activeSeconds)}</div>
              </button>
            {/each}
          </div>
        {/if}
      </div>
    </section>

    <!-- Looked-Up Words Leaderboard & Revision Bank -->
    <section class="im-vocab-section">
      <div class="vocab-toolbar">
        <div class="vt-left">
          <h2>Looked-Up Words & Revision Bank</h2>
          <div class="status-tabs">
            <button
              type="button"
              class="st-tab"
              class:active={statusFilter === "all"}
              onclick={() => (statusFilter = "all")}
            >
              All <span class="cnt">{overview.uniqueWordsCount}</span>
            </button>
            <button
              type="button"
              class="st-tab learning"
              class:active={statusFilter === "learning"}
              onclick={() => (statusFilter = "learning")}
            >
              ● Tracked <span class="cnt">{overview.learningWordsCount}</span>
            </button>
            <button
              type="button"
              class="st-tab known"
              class:active={statusFilter === "known"}
              onclick={() => (statusFilter = "known")}
            >
              ✓ Known <span class="cnt">{overview.knownWordsCount}</span>
            </button>
            {#if overview.ignoredWordsCount > 0}
              <button
                type="button"
                class="st-tab"
                class:active={statusFilter === "ignored"}
                onclick={() => (statusFilter = "ignored")}
              >
                Ignored <span class="cnt">{overview.ignoredWordsCount}</span>
              </button>
            {/if}
          </div>
        </div>

        <div class="vt-right">
          <div class="search-box">
            <Icon name="search" size={14} />
            <input
              type="text"
              placeholder="Search kanji, reading, or meaning..."
              bind:value={searchQuery}
            />
            {#if searchQuery}
              <button type="button" class="clear-btn" onclick={() => (searchQuery = "")}>✕</button>
            {/if}
          </div>

          <select class="sort-select" bind:value={sortBy} aria-label="Sort words">
            <option value="count">Most Looked Up (20× → 1×)</option>
            <option value="recent">Recently Looked Up</option>
            <option value="oldest">First Encountered</option>
          </select>
        </div>
      </div>

      {#if filteredWords.length === 0}
        <div class="vocab-empty-state">
          <div class="ve-icon"><Icon name="languages" size={32} /></div>
          <h3>
            {overview.words.length === 0
              ? "No words looked up yet"
              : "No words match your current filter"}
          </h3>
          <p>
            {#if overview.words.length === 0}
              Pause any episode with Japanese subtitles in Kura's Built-In Player and hover or click on a word to look it up with Yomitan. Every word you look up is automatically counted, ranked by lookup frequency, and saved here with its exact anime subtitle sentence for revision!
            {:else}
              Try clearing your search query or switching status tabs above.
            {/if}
          </p>
        </div>
      {:else}
        <div class="vocab-split-layout">
          <!-- Left Leaderboard List -->
          <div class="vocab-list">
            {#each filteredWords as word, idx (word.id)}
              {@const isSelected = selectedWord?.id === word.id}
              {@const isHidden = flashcardMode && !revealedIds.has(word.id)}
              {@const latestCtx = word.contexts[0] ?? null}
              <div
                class="vocab-row"
                class:selected={isSelected}
                class:status-known={word.status === "known"}
                role="button"
                tabindex="0"
                onclick={() => (selectedWordId = word.id)}
                onkeydown={(e) => e.key === "Enter" && (selectedWordId = word.id)}
              >
                <!-- Rank & Frequency Counter -->
                <div class="vr-rank-col">
                  <span class="vr-rank">#{idx + 1}</span>
                  <span
                    class="vr-count-badge"
                    class:leeches={word.lookupCount >= 5}
                    class:warm={word.lookupCount >= 2 && word.lookupCount < 5}
                    title={`Looked up ${word.lookupCount} time${word.lookupCount === 1 ? "" : "s"}`}
                  >
                    {word.lookupCount}×
                  </span>
                </div>

                <!-- Headword, Furigana & Meaning -->
                <div class="vr-main-col">
                  <div class="vr-head-line">
                    <div class="vr-headword">
                      {#if word.furigana.length > 0}
                        {#each word.furigana as seg}
                          {#if seg.reading && !isHidden}
                            <ruby>{seg.text}<rt>{seg.reading}</rt></ruby>
                          {:else}
                            <span>{seg.text}</span>
                          {/if}
                        {/each}
                      {:else}
                        <span>{word.expression}</span>
                      {/if}
                    </div>

                    {#if word.matchedForms.some((f) => f !== word.expression)}
                      <div class="vr-surface-forms">
                        {#each word.matchedForms.filter((f) => f !== word.expression).slice(0, 2) as form}
                          <span class="surface-chip">seen as {form}</span>
                        {/each}
                      </div>
                    {/if}

                    {#if word.frequencies.length > 0}
                      <span class="vr-freq-pill">Freq: {word.frequencies[0]}</span>
                    {/if}
                  </div>

                  {#if isHidden}
                    <button
                      type="button"
                      class="reveal-meaning-btn"
                      onclick={(e) => toggleReveal(word.id, e)}
                    >
                      Click to reveal reading & meaning
                    </button>
                  {:else}
                    <div class="vr-meaning">
                      {word.glossaries.slice(0, 2).join(" · ") || "No definition recorded"}
                    </div>
                  {/if}

                  {#if latestCtx}
                    <div class="vr-context-preview">
                      <span class="ctx-quote">「{latestCtx.sentence.replace(/\n/g, " ")}」</span>
                      {#if latestCtx.animeTitle}
                        <span class="ctx-anime">
                          — {latestCtx.animeTitle}{latestCtx.epKey ? ` · Ep ${latestCtx.epKey}` : ""}
                        </span>
                      {/if}
                    </div>
                  {/if}
                </div>

                <!-- Quick Status Controls -->
                <div class="vr-actions-col">
                  <button
                    type="button"
                    class="vr-icon-btn"
                    title="Pronounce"
                    onclick={(e) => speakJapanese(word.reading || word.expression, e)}
                  >
                    🔊
                  </button>
                  <button
                    type="button"
                    class="vr-status-btn learning"
                    class:active={word.status === "learning"}
                    title="Mark as Tracked / Learning"
                    onclick={(e) => updateWordStatus(word, "learning", e)}
                  >
                    {word.status === "learning" ? "● Tracked" : "+ Track"}
                  </button>
                  <button
                    type="button"
                    class="vr-status-btn known"
                    class:active={word.status === "known"}
                    title="Mark as Known"
                    onclick={(e) => updateWordStatus(word, "known", e)}
                  >
                    {word.status === "known" ? "✓ Known" : "Known"}
                  </button>
                </div>
              </div>
            {/each}
          </div>

          <!-- Right Yomitan Revision Inspector -->
          {#if selectedWord}
            {@const detailHidden = flashcardMode && !revealedIds.has(selectedWord.id)}
            <aside class="vocab-inspector">
              <div class="vi-top">
                <div class="vi-badges">
                  <span
                    class="vi-lookup-pill"
                    class:leeches={selectedWord.lookupCount >= 5}
                  >
                    Looked up {selectedWord.lookupCount}×
                  </span>
                  <span class="vi-seen-meta">
                    Last looked up {formatRelativeTime(selectedWord.lastSeenAt)}
                  </span>
                </div>

                <div class="vi-top-btns">
                  <button
                    type="button"
                    class="vi-audio-btn"
                    title="Pronounce word"
                    onclick={() => speakJapanese(selectedWord.reading || selectedWord.expression)}
                  >
                    🔊 Pronounce
                  </button>
                  <button
                    type="button"
                    class="vi-delete-btn"
                    title="Remove word from bank"
                    onclick={() => removeWord(selectedWord)}
                  >
                    <Icon name="trash" size={14} />
                  </button>
                </div>
              </div>

              <!-- Big Headword with Furigana -->
              <div class="vi-headword-block">
                <div class="vi-headword">
                  {#if selectedWord.furigana.length > 0}
                    {#each selectedWord.furigana as seg}
                      {#if seg.reading && !detailHidden}
                        <ruby>{seg.text}<rt>{seg.reading}</rt></ruby>
                      {:else}
                        <span>{seg.text}</span>
                      {/if}
                    {/each}
                  {:else}
                    <span>{selectedWord.expression}</span>
                  {/if}
                </div>

                {#if !detailHidden && selectedWord.reading && selectedWord.reading !== selectedWord.expression}
                  <div class="vi-kana-sub">【{selectedWord.reading}】</div>
                {/if}
              </div>

              <!-- Migaku-style Status Switcher -->
              <div class="vi-status-switcher">
                <button
                  type="button"
                  class="vis-btn learning"
                  class:active={selectedWord.status === "learning"}
                  onclick={() => updateWordStatus(selectedWord, "learning")}
                >
                  ● Tracked (Learning)
                </button>
                <button
                  type="button"
                  class="vis-btn known"
                  class:active={selectedWord.status === "known"}
                  onclick={() => updateWordStatus(selectedWord, "known")}
                >
                  ✓ Known
                </button>
                <button
                  type="button"
                  class="vis-btn ignored"
                  class:active={selectedWord.status === "ignored"}
                  onclick={() => updateWordStatus(selectedWord, "ignored")}
                >
                  ✕ Ignore
                </button>
              </div>

              <!-- Pitch Accent, Inflections & Tags -->
              {#if !detailHidden}
                <div class="vi-meta-tags">
                  {#if selectedWord.pitches.length > 0}
                    {#each selectedWord.pitches.slice(0, 3) as pitch}
                      {@const moras = splitMoras(pitch.reading || selectedWord.reading || selectedWord.expression)}
                      <span class="vi-pitch" title="Pitch accent [{pitch.downstep}]">
                        {#each moras as mora, mIdx}
                          {@const high = isMoraHigh(mIdx, pitch.downstep)}
                          {@const nextHigh = mIdx + 1 < moras.length ? isMoraHigh(mIdx + 1, pitch.downstep) : false}
                          <span
                            class="mora"
                            class:high
                            class:low={!high}
                            class:drop={high && !nextHigh && pitch.downstep === mIdx + 1}
                          >{mora}</span>
                        {/each}
                        <sup class="pitch-num">[{pitch.downstep}]</sup>
                      </span>
                    {/each}
                  {/if}

                  {#each selectedWord.frequencies as freq}
                    <span class="vi-tag freq">Freq: {freq}</span>
                  {/each}

                  {#each selectedWord.posTags.split(/\s+/).filter(Boolean) as tag}
                    <span class="vi-tag pos">{tag}</span>
                  {/each}
                </div>

                {#if selectedWord.matchedForms.length > 0}
                  <div class="vi-forms-box">
                    <span class="forms-label">Encountered inflections:</span>
                    {#each selectedWord.matchedForms as form}
                      <span class="form-pill">{form}</span>
                    {/each}
                  </div>
                {/if}
              {/if}

              <!-- Yomitan Glossaries -->
              <div class="vi-section">
                <h3>Meanings & Definitions</h3>
                {#if detailHidden}
                  <button
                    type="button"
                    class="vi-flashcard-reveal"
                    onclick={() => toggleReveal(selectedWord.id)}
                  >
                    <Icon name="eye" size={16} />
                    <span>Reveal Reading, Pitch Accent & Meanings</span>
                  </button>
                {:else}
                  <ol class="vi-glossary-list">
                    {#each selectedWord.glossaries as gloss}
                      <li>{gloss}</li>
                    {/each}
                  </ol>
                {/if}
              </div>

              <!-- Anime Context Sentences -->
              <div class="vi-section">
                <h3>
                  Anime Context Sentences
                  <span class="sec-count">{selectedWord.contexts.length}</span>
                </h3>
                {#if selectedWord.contexts.length === 0}
                  <p class="vi-muted">No subtitle sentence recorded for this word yet.</p>
                {:else}
                  <div class="vi-contexts-list">
                    {#each selectedWord.contexts as ctx (ctx.id)}
                      {@const parts = highlightTargetInSentence(
                        ctx.sentence,
                        ctx.matchedSurface,
                        selectedWord.expression
                      )}
                      {@const cover = img(ctx.coverPath, ctx.coverUrl)}
                      <div class="vi-context-card">
                        <div class="vic-sentence">
                          {#if parts}
                            <span>{parts.before}</span><mark>{parts.match}</mark><span>{parts.after}</span>
                          {:else}
                            <span>{ctx.sentence}</span>
                          {/if}
                        </div>

                        <div class="vic-footer">
                          <div class="vic-source">
                            {#if cover}
                              <img src={cover} alt="" class="vic-thumb" />
                            {/if}
                            <span>
                              {ctx.animeTitle || "Anime Episode"}
                              {#if ctx.epKey}
                                · Ep {ctx.epKey}
                              {/if}
                              {#if ctx.positionSec !== null}
                                · {formatTimestamp(ctx.positionSec)}
                              {/if}
                            </span>
                          </div>

                          <div class="vic-actions">
                            <button
                              type="button"
                              class="vic-btn"
                              title="Pronounce sentence"
                              onclick={() => speakJapanese(ctx.sentence)}
                            >
                              🔊
                            </button>
                            {#if ctx.anilistId}
                              <button
                                type="button"
                                class="vic-btn"
                                title="Open Anime Page"
                                onclick={() => goto(`/anime/${ctx.anilistId}`)}
                              >
                                Open Anime →
                              </button>
                            {/if}
                          </div>
                        </div>
                      </div>
                    {/each}
                  </div>
                {/if}
              </div>
            </aside>
          {/if}
        </div>
      {/if}
    </section>
  {/if}
</div>

<!-- Daily Immersion Goal Modal -->
{#if goalModalOpen}
  <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
  <div class="im-modal-backdrop" onclick={() => (goalModalOpen = false)}>
    <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
    <div class="im-modal" onclick={(e) => e.stopPropagation()}>
      <h3>Daily Immersion Goal</h3>
      <p>Set how many minutes of active Japanese immersion you aim for each day.</p>
      <div class="preset-chips">
        {#each [30, 45, 60, 90, 120, 180] as mins}
          <button
            type="button"
            class="preset-chip"
            class:active={goalInputMinutes === mins}
            onclick={() => (goalInputMinutes = mins)}
          >
            {mins}m
          </button>
        {/each}
      </div>
      <label class="modal-field">
        <span>Minutes per day</span>
        <input type="number" min="5" max="720" bind:value={goalInputMinutes} />
      </label>
      <div class="modal-actions">
        <button type="button" class="im-btn" onclick={() => (goalModalOpen = false)}>Cancel</button>
        <button type="button" class="im-btn primary" onclick={saveDailyGoal}>Save Goal</button>
      </div>
    </div>
  </div>
{/if}

<!-- Manual Immersion Time Log Modal -->
{#if logModalOpen}
  <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
  <div class="im-modal-backdrop" onclick={() => (logModalOpen = false)}>
    <!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
    <div class="im-modal" onclick={(e) => e.stopPropagation()}>
      <h3>Log Immersion Time</h3>
      <p>
        Active playback in Kura is tracked automatically, or you can manually log additional Japanese immersion (e.g. external watching, manga, or light novels).
      </p>
      <div class="preset-chips">
        {#each [15, 24, 45, 60, 90] as mins}
          <button
            type="button"
            class="preset-chip"
            class:active={logMinutes === mins}
            onclick={() => (logMinutes = mins)}
          >
            +{mins}m
          </button>
        {/each}
      </div>
      <label class="modal-field">
        <span>Duration (minutes)</span>
        <input type="number" min="1" max="720" bind:value={logMinutes} />
      </label>
      <label class="modal-field">
        <span>Link to Anime in Library (optional)</span>
        <select bind:value={logAnilistId}>
          <option value={null}>— General / External Immersion —</option>
          {#each app.cards as card (card.anilistId)}
            <option value={card.anilistId}>
              {card.titleEnglish || card.titleRomaji}
            </option>
          {/each}
        </select>
      </label>
      <label class="modal-field">
        <span>Label / Note (optional)</span>
        <input
          type="text"
          placeholder="e.g. Episode rewatch, Visual Novel, Reading..."
          bind:value={logLabel}
        />
      </label>
      <div class="modal-actions">
        <button type="button" class="im-btn" onclick={() => (logModalOpen = false)}>Cancel</button>
        <button type="button" class="im-btn primary" onclick={submitManualLog} disabled={submittingLog}>
          {submittingLog ? "Logging..." : `Log +${logMinutes}m`}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .immersion-page {
    padding: 28px 34px 64px;
    max-width: 1480px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 22px;
  }

  .im-header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 20px;
    flex-wrap: wrap;
  }

  .im-eyebrow {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 800;
    letter-spacing: 0.08em;
    color: var(--accent);
    margin-bottom: 4px;
  }

  .im-title-group h1 {
    font-size: 28px;
    font-weight: 800;
    letter-spacing: -0.02em;
    margin: 0;
    color: var(--fg);
  }

  .im-subtitle {
    margin: 4px 0 0;
    font-size: 13.5px;
    color: var(--fg-muted);
  }

  .im-header-actions {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .im-btn {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 8px 14px;
    border-radius: 9px;
    border: 1px solid var(--border-strong);
    background: var(--bg-card);
    color: var(--fg);
    font-size: 12.5px;
    font-weight: 700;
    cursor: pointer;
    transition: all 130ms ease;
  }

  .im-btn:hover {
    background: var(--bg-hover);
    border-color: var(--accent);
  }

  .im-btn.active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
  }

  .im-btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }

  .im-btn.primary:hover {
    filter: brightness(1.08);
  }

  .im-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 80px 20px;
    color: var(--fg-muted);
    font-size: 14px;
  }

  /* Metrics Grid */
  .im-metrics-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 14px;
  }

  @media (max-width: 1120px) {
    .im-metrics-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }

  .im-metric-card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 16px 18px;
    display: flex;
    align-items: center;
    gap: 14px;
  }

  .goal-ring-wrap {
    position: relative;
    width: 64px;
    height: 64px;
    flex-shrink: 0;
  }

  .goal-ring {
    width: 64px;
    height: 64px;
    transform: rotate(-90deg);
  }

  .ring-bg {
    fill: none;
    stroke: var(--border-strong);
    stroke-width: 6.5;
  }

  .ring-fg {
    fill: none;
    stroke: var(--accent);
    stroke-width: 6.5;
    stroke-linecap: round;
    transition: stroke-dashoffset 300ms ease;
  }

  .ring-pct {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    font-size: 12px;
    font-weight: 800;
    color: var(--fg);
  }

  .metric-info {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .metric-label {
    font-size: 10.5px;
    font-weight: 800;
    letter-spacing: 0.08em;
    color: var(--fg-subtle);
  }

  .metric-val {
    font-size: 24px;
    font-weight: 800;
    color: var(--fg);
    line-height: 1.2;
    font-variant-numeric: tabular-nums;
  }

  .metric-val small {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg-muted);
  }

  .metric-sub {
    font-size: 12px;
    color: var(--fg-muted);
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .inline-link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    font-size: 11.5px;
    font-weight: 700;
    cursor: pointer;
    text-decoration: underline;
  }

  .streak-pill {
    font-weight: 700;
    color: var(--fg-muted);
  }

  .streak-pill.lit {
    color: #f4a261;
  }

  .vocab-pills {
    gap: 10px;
  }

  .v-dot.learning {
    color: #f4a261;
    font-weight: 700;
  }

  .v-dot.known {
    color: #34d399;
    font-weight: 700;
  }

  /* Analytics Row */
  .im-analytics-row {
    display: grid;
    grid-template-columns: 1.65fr 1fr;
    gap: 16px;
  }

  @media (max-width: 1024px) {
    .im-analytics-row {
      grid-template-columns: 1fr;
    }
  }

  .im-panel {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 18px 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .panel-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .panel-head h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 800;
    color: var(--fg);
  }

  .panel-sub {
    margin: 2px 0 0;
    font-size: 12px;
    color: var(--fg-muted);
  }

  .seg-toggle {
    display: inline-flex;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 2px;
  }

  .seg-toggle button {
    background: transparent;
    border: none;
    color: var(--fg-muted);
    font-size: 11.5px;
    font-weight: 700;
    padding: 4px 10px;
    border-radius: 6px;
    cursor: pointer;
  }

  .seg-toggle button.active {
    background: var(--bg-card);
    color: var(--fg);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.2);
  }

  /* Bar Chart */
  .bar-chart-wrap {
    position: relative;
    height: 175px;
    padding-top: 18px;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
  }

  .goal-line {
    position: absolute;
    left: 0;
    right: 0;
    border-top: 1px dashed rgba(244, 162, 97, 0.45);
    pointer-events: none;
    z-index: 2;
  }

  .goal-line span {
    position: absolute;
    right: 0;
    top: -16px;
    font-size: 10px;
    font-weight: 700;
    color: #f4a261;
    background: var(--bg-card);
    padding: 1px 5px;
    border-radius: 4px;
  }

  .bars-grid {
    display: grid;
    grid-template-columns: repeat(14, minmax(0, 1fr));
    gap: 8px;
    height: 100%;
    align-items: end;
  }

  .bars-grid.dense {
    grid-template-columns: repeat(30, minmax(0, 1fr));
    gap: 4px;
  }

  .bar-col {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
  }

  .bar-track {
    width: 100%;
    max-width: 24px;
    height: 128px;
    background: rgba(255, 255, 255, 0.03);
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    align-items: center;
    position: relative;
    overflow: visible;
  }

  .bar-fill {
    width: 100%;
    border-radius: 5px;
    background: rgba(244, 162, 97, 0.55);
    transition: height 220ms ease;
  }

  .bar-fill.met-goal {
    background: var(--accent);
  }

  .bar-lookup-pip {
    font-size: 9.5px;
    font-weight: 800;
    color: var(--fg-muted);
    margin-bottom: 3px;
  }

  .bar-date {
    font-size: 10px;
    font-weight: 600;
    color: var(--fg-subtle);
    font-variant-numeric: tabular-nums;
  }

  /* Top Immersed Anime */
  .top-anime-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .top-anime-row {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 7px 10px;
    border-radius: 10px;
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--fg);
    text-align: left;
    cursor: pointer;
    transition: border-color 120ms ease, background 120ms ease;
  }

  .top-anime-row:hover {
    border-color: var(--accent);
    background: var(--bg-hover);
  }

  .ta-cover {
    width: 34px;
    height: 46px;
    border-radius: 6px;
    overflow: hidden;
    flex-shrink: 0;
    background: var(--bg-card);
  }

  .ta-cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .ta-fallback {
    width: 100%;
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--fg-subtle);
  }

  .ta-info {
    flex: 1;
    min-width: 0;
  }

  .ta-title {
    font-size: 13px;
    font-weight: 700;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ta-meta {
    font-size: 11.5px;
    color: var(--fg-muted);
    margin-top: 2px;
  }

  .ta-time {
    font-size: 13px;
    font-weight: 800;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }

  .panel-empty {
    font-size: 12.5px;
    color: var(--fg-muted);
    padding: 28px 12px;
    text-align: center;
    line-height: 1.5;
  }

  /* Vocabulary Section */
  .im-vocab-section {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .vocab-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    flex-wrap: wrap;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 14px 18px;
  }

  .vt-left {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
  }

  .vt-left h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 800;
  }

  .status-tabs {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .st-tab {
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--fg-muted);
    font-size: 12px;
    font-weight: 700;
    padding: 6px 11px;
    border-radius: 8px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    transition: all 120ms ease;
  }

  .st-tab:hover {
    color: var(--fg);
    border-color: var(--border-strong);
  }

  .st-tab.active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
  }

  .st-tab .cnt {
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 99px;
    background: rgba(255, 255, 255, 0.07);
  }

  .vt-right {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-radius: 9px;
    padding: 6px 11px;
    min-width: 240px;
    color: var(--fg-muted);
  }

  .search-box input {
    background: none;
    border: none;
    outline: none;
    color: var(--fg);
    font-size: 12.5px;
    width: 100%;
  }

  .clear-btn {
    background: none;
    border: none;
    color: var(--fg-muted);
    cursor: pointer;
    font-size: 11px;
    padding: 0 2px;
  }

  .sort-select {
    background: var(--bg);
    border: 1px solid var(--border-strong);
    color: var(--fg);
    font-size: 12.5px;
    font-weight: 600;
    padding: 7px 11px;
    border-radius: 9px;
    cursor: pointer;
  }

  .vocab-empty-state {
    background: var(--bg-card);
    border: 1px dashed var(--border-strong);
    border-radius: 14px;
    padding: 56px 28px;
    text-align: center;
    max-width: 640px;
    margin: 12px auto;
  }

  .ve-icon {
    color: var(--accent);
    margin-bottom: 10px;
  }

  .vocab-empty-state h3 {
    margin: 0 0 8px;
    font-size: 18px;
    font-weight: 800;
  }

  .vocab-empty-state p {
    margin: 0;
    font-size: 13.5px;
    line-height: 1.6;
    color: var(--fg-muted);
  }

  /* Split Leaderboard + Inspector */
  .vocab-split-layout {
    display: grid;
    grid-template-columns: 1.25fr 1fr;
    gap: 16px;
    align-items: start;
  }

  @media (max-width: 1080px) {
    .vocab-split-layout {
      grid-template-columns: 1fr;
    }
  }

  .vocab-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .vocab-row {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 13px 16px;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    cursor: pointer;
    transition: all 120ms ease;
  }

  .vocab-row:hover {
    border-color: var(--border-strong);
    background: var(--bg-hover);
  }

  .vocab-row.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .vocab-row.status-known {
    opacity: 0.86;
  }

  .vr-rank-col {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    min-width: 48px;
    flex-shrink: 0;
  }

  .vr-rank {
    font-size: 11px;
    font-weight: 700;
    color: var(--fg-subtle);
    font-variant-numeric: tabular-nums;
  }

  .vr-count-badge {
    font-size: 13px;
    font-weight: 800;
    padding: 3px 9px;
    border-radius: 99px;
    background: rgba(255, 255, 255, 0.07);
    color: var(--fg);
    border: 1px solid var(--border-strong);
    font-variant-numeric: tabular-nums;
  }

  .vr-count-badge.warm {
    background: rgba(244, 162, 97, 0.16);
    border-color: rgba(244, 162, 97, 0.45);
    color: #f4a261;
  }

  .vr-count-badge.leeches {
    background: rgba(231, 111, 81, 0.22);
    border-color: rgba(231, 111, 81, 0.65);
    color: #ff8a6d;
  }

  .vr-main-col {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .vr-head-line {
    display: flex;
    align-items: baseline;
    gap: 9px;
    flex-wrap: wrap;
  }

  .vr-headword {
    font-size: 21px;
    font-weight: 800;
    color: var(--fg);
    line-height: 1.3;
  }

  .vr-headword ruby {
    ruby-align: center;
  }

  .vr-headword rt {
    font-size: 10.5px;
    font-weight: 600;
    color: #f4a261;
  }

  .vr-surface-forms {
    display: inline-flex;
    gap: 5px;
    flex-wrap: wrap;
  }

  .surface-chip {
    font-size: 11px;
    font-weight: 600;
    padding: 1px 7px;
    border-radius: 6px;
    background: rgba(244, 162, 97, 0.12);
    color: #f4a261;
  }

  .vr-freq-pill {
    font-size: 10.5px;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: 5px;
    background: rgba(42, 157, 143, 0.18);
    color: #5eead4;
  }

  .vr-meaning {
    font-size: 13px;
    color: var(--fg-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .reveal-meaning-btn {
    align-self: flex-start;
    background: var(--bg);
    border: 1px dashed var(--border-strong);
    color: var(--accent);
    font-size: 11.5px;
    font-weight: 700;
    padding: 3px 9px;
    border-radius: 6px;
    cursor: pointer;
  }

  .vr-context-preview {
    font-size: 11.5px;
    color: var(--fg-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ctx-quote {
    color: var(--fg-muted);
  }

  .vr-actions-col {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }

  .vr-icon-btn {
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--fg);
    width: 30px;
    height: 28px;
    border-radius: 7px;
    cursor: pointer;
    font-size: 12px;
  }

  .vr-status-btn {
    background: var(--bg);
    border: 1px solid var(--border-strong);
    color: var(--fg-muted);
    font-size: 11px;
    font-weight: 700;
    padding: 5px 9px;
    border-radius: 7px;
    cursor: pointer;
    transition: all 120ms ease;
  }

  .vr-status-btn.learning.active {
    background: rgba(231, 111, 81, 0.2);
    border-color: rgba(231, 111, 81, 0.65);
    color: #ff9b82;
  }

  .vr-status-btn.known.active {
    background: rgba(42, 157, 143, 0.22);
    border-color: rgba(42, 157, 143, 0.65);
    color: #5eead4;
  }

  /* Right Inspector Drawer */
  .vocab-inspector {
    position: sticky;
    top: 20px;
    background: var(--bg-card);
    border: 1px solid var(--border-strong);
    border-radius: 14px;
    padding: 20px 22px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .vi-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
  }

  .vi-badges {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .vi-lookup-pill {
    font-size: 12px;
    font-weight: 800;
    padding: 4px 10px;
    border-radius: 99px;
    background: rgba(244, 162, 97, 0.18);
    color: #f4a261;
    border: 1px solid rgba(244, 162, 97, 0.45);
  }

  .vi-lookup-pill.leeches {
    background: rgba(231, 111, 81, 0.24);
    color: #ff9b82;
    border-color: rgba(231, 111, 81, 0.6);
  }

  .vi-seen-meta {
    font-size: 11.5px;
    color: var(--fg-subtle);
  }

  .vi-top-btns {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .vi-audio-btn {
    background: var(--bg);
    border: 1px solid var(--border-strong);
    color: var(--fg);
    font-size: 12px;
    font-weight: 700;
    padding: 5px 10px;
    border-radius: 8px;
    cursor: pointer;
  }

  .vi-delete-btn {
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--fg-muted);
    width: 30px;
    height: 30px;
    border-radius: 8px;
    display: grid;
    place-items: center;
    cursor: pointer;
  }

  .vi-delete-btn:hover {
    color: #ef4444;
    border-color: rgba(239, 68, 68, 0.5);
  }

  .vi-headword-block {
    display: flex;
    align-items: baseline;
    gap: 12px;
    flex-wrap: wrap;
  }

  .vi-headword {
    font-size: 34px;
    font-weight: 800;
    color: var(--fg);
    line-height: 1.3;
  }

  .vi-headword rt {
    font-size: 13px;
    font-weight: 600;
    color: #f4a261;
  }

  .vi-kana-sub {
    font-size: 16px;
    font-weight: 600;
    color: var(--fg-muted);
  }

  .vi-status-switcher {
    display: grid;
    grid-template-columns: 1.2fr 1fr 0.8fr;
    gap: 8px;
  }

  .vis-btn {
    background: var(--bg);
    border: 1px solid var(--border-strong);
    color: var(--fg-muted);
    font-size: 12px;
    font-weight: 700;
    padding: 8px 10px;
    border-radius: 9px;
    cursor: pointer;
    transition: all 120ms ease;
  }

  .vis-btn.learning.active {
    background: rgba(231, 111, 81, 0.22);
    border-color: rgba(231, 111, 81, 0.7);
    color: #ff9b82;
  }

  .vis-btn.known.active {
    background: rgba(42, 157, 143, 0.24);
    border-color: rgba(42, 157, 143, 0.7);
    color: #5eead4;
  }

  .vis-btn.ignored.active {
    background: rgba(255, 255, 255, 0.14);
    border-color: var(--border-strong);
    color: var(--fg);
  }

  .vi-meta-tags {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .vi-tag {
    font-size: 11px;
    font-weight: 700;
    padding: 2px 8px;
    border-radius: 6px;
  }

  .vi-tag.freq {
    background: rgba(42, 157, 143, 0.2);
    color: #5eead4;
  }

  .vi-tag.pos {
    background: rgba(244, 162, 97, 0.16);
    color: #f4a261;
  }

  .vi-pitch {
    display: inline-flex;
    align-items: center;
    padding: 2px 8px;
    border-radius: 6px;
    background: var(--bg);
    border: 1px solid var(--border);
    font-size: 12.5px;
    font-weight: 700;
  }

  .vi-pitch .mora {
    position: relative;
    padding: 0 1px;
  }

  .vi-pitch .mora.high {
    border-top: 2px solid #f4a261;
  }

  .vi-pitch .mora.low {
    border-bottom: 2px solid rgba(255, 255, 255, 0.25);
  }

  .vi-pitch .mora.drop {
    border-right: 2px solid #f4a261;
  }

  .pitch-num {
    font-size: 9.5px;
    color: #f4a261;
    margin-left: 3px;
  }

  .vi-forms-box {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    font-size: 12px;
    color: var(--fg-muted);
  }

  .form-pill {
    background: var(--bg);
    border: 1px solid var(--border);
    padding: 2px 8px;
    border-radius: 6px;
    font-weight: 700;
    color: var(--fg);
  }

  .vi-section h3 {
    margin: 0 0 8px;
    font-size: 12px;
    font-weight: 800;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-subtle);
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .sec-count {
    font-size: 10.5px;
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--bg);
    color: var(--fg-muted);
  }

  .vi-glossary-list {
    margin: 0;
    padding-left: 20px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 14px;
    line-height: 1.5;
    color: var(--fg);
  }

  .vi-flashcard-reveal {
    width: 100%;
    padding: 16px;
    border-radius: 10px;
    border: 1px dashed var(--accent);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 13px;
    font-weight: 700;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    cursor: pointer;
  }

  .vi-contexts-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .vi-context-card {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .vic-sentence {
    font-size: 15px;
    font-weight: 700;
    line-height: 1.5;
    color: var(--fg);
  }

  .vic-sentence mark {
    background: rgba(244, 162, 97, 0.25);
    color: #f4a261;
    padding: 0 3px;
    border-radius: 4px;
  }

  .vic-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: 11.5px;
    color: var(--fg-muted);
  }

  .vic-source {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
  }

  .vic-thumb {
    width: 18px;
    height: 24px;
    border-radius: 3px;
    object-fit: cover;
  }

  .vic-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .vic-btn {
    background: var(--bg-card);
    border: 1px solid var(--border);
    color: var(--fg);
    font-size: 11px;
    font-weight: 700;
    padding: 3px 8px;
    border-radius: 6px;
    cursor: pointer;
  }

  .vic-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  /* Modals */
  .im-modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9500;
    background: rgba(0, 0, 0, 0.72);
    backdrop-filter: blur(6px);
    display: grid;
    place-items: center;
    padding: 20px;
  }

  .im-modal {
    width: 420px;
    max-width: 100%;
    background: var(--bg-card);
    border: 1px solid var(--border-strong);
    border-radius: 16px;
    padding: 22px 24px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .im-modal h3 {
    margin: 0;
    font-size: 18px;
    font-weight: 800;
  }

  .im-modal p {
    margin: 0;
    font-size: 13px;
    color: var(--fg-muted);
    line-height: 1.5;
  }

  .preset-chips {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .preset-chip {
    background: var(--bg);
    border: 1px solid var(--border-strong);
    color: var(--fg);
    font-size: 12px;
    font-weight: 700;
    padding: 6px 11px;
    border-radius: 8px;
    cursor: pointer;
  }

  .preset-chip.active {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent);
  }

  .modal-field {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    font-weight: 700;
    color: var(--fg-muted);
  }

  .modal-field input,
  .modal-field select {
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    padding: 8px 11px;
    color: var(--fg);
    font-size: 13px;
  }

  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>

