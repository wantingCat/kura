//! Metadata providers: AniList (series), ani.zip (episodes), Jikan/MAL (episode fallback).

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

/// Simple "minimum interval between requests" limiter.
pub struct RateLimiter {
    interval: std::sync::Mutex<Duration>,
    last: Mutex<Option<Instant>>,
}

impl RateLimiter {
    pub fn new(interval: Duration) -> Self {
        Self { interval: std::sync::Mutex::new(interval), last: Mutex::new(None) }
    }

    pub fn set_interval(&self, d: Duration) {
        *self.interval.lock().unwrap() = d;
    }

    pub async fn wait(&self) {
        let mut last = self.last.lock().await;
        let interval = *self.interval.lock().unwrap();
        if let Some(t) = *last {
            let elapsed = t.elapsed();
            if elapsed < interval {
                tokio::time::sleep(interval - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }
}

pub struct Providers {
    pub http: reqwest::Client,
    anilist_rl: RateLimiter,
    anizip_rl: RateLimiter,
    jikan_rl: RateLimiter,
}

impl Providers {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("Kura/", env!("CARGO_PKG_VERSION"), " (+https://github.com/kura-app/kura)"))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("http client");
        Self {
            http,
            // AniList is currently limited to ~30 req/min; adjusted from response headers.
            anilist_rl: RateLimiter::new(Duration::from_millis(2100)),
            anizip_rl: RateLimiter::new(Duration::from_millis(250)),
            jikan_rl: RateLimiter::new(Duration::from_millis(1100)),
        }
    }
}

// ---------------------------------------------------------------------------
// AniList
// ---------------------------------------------------------------------------

const MEDIA_FIELDS: &str = r#"
  id idMal type format status episodes duration season seasonYear averageScore synonyms genres
  title { romaji english native }
  startDate { year month day }
  endDate { year month day }
  description(asHtml: false)
  coverImage { extraLarge large color }
  bannerImage
  tags { name rank isMediaSpoiler }
  studios(isMain: true) { nodes { name } }
  nextAiringEpisode { episode airingAt }
  relations {
    edges {
      relationType(version: 2)
      node { id type format status episodes seasonYear title { romaji english } coverImage { large } }
    }
  }
"#;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlTitle {
    pub romaji: Option<String>,
    pub english: Option<String>,
    pub native: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlDate {
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub day: Option<i64>,
}

impl AlDate {
    pub fn to_string_opt(&self) -> Option<String> {
        let y = self.year?;
        Some(match (self.month, self.day) {
            (Some(m), Some(d)) => format!("{y:04}-{m:02}-{d:02}"),
            (Some(m), None) => format!("{y:04}-{m:02}"),
            _ => format!("{y:04}"),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlCover {
    pub extra_large: Option<String>,
    pub large: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlTag {
    pub name: String,
    pub rank: Option<i64>,
    pub is_media_spoiler: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlStudioNode {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlStudios {
    pub nodes: Vec<AlStudioNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlAiring {
    pub episode: i64,
    pub airing_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlRelNode {
    pub id: i64,
    #[serde(rename = "type")]
    pub media_type: Option<String>,
    pub format: Option<String>,
    pub status: Option<String>,
    pub episodes: Option<i64>,
    pub season_year: Option<i64>,
    pub title: AlTitle,
    pub cover_image: Option<AlCover>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlRelEdge {
    pub relation_type: String,
    pub node: AlRelNode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlRelations {
    pub edges: Vec<AlRelEdge>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlMedia {
    pub id: i64,
    pub id_mal: Option<i64>,
    #[serde(rename = "type")]
    pub media_type: Option<String>,
    pub format: Option<String>,
    pub status: Option<String>,
    pub episodes: Option<i64>,
    pub duration: Option<i64>,
    pub season: Option<String>,
    pub season_year: Option<i64>,
    pub average_score: Option<i64>,
    #[serde(default)]
    pub synonyms: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    pub title: AlTitle,
    pub start_date: Option<AlDate>,
    pub end_date: Option<AlDate>,
    pub description: Option<String>,
    pub cover_image: Option<AlCover>,
    pub banner_image: Option<String>,
    #[serde(default)]
    pub tags: Vec<AlTag>,
    pub studios: Option<AlStudios>,
    pub next_airing_episode: Option<AlAiring>,
    pub relations: Option<AlRelations>,
}

impl AlMedia {
    pub fn all_titles(&self) -> Vec<String> {
        let mut v = Vec::new();
        for t in [&self.title.romaji, &self.title.english, &self.title.native].into_iter().flatten() {
            v.push(t.clone());
        }
        v.extend(self.synonyms.iter().cloned());
        v
    }
}

impl Providers {
    async fn anilist_query(&self, query: &str, variables: Value) -> Result<Value> {
        let body = json!({ "query": query, "variables": variables });
        for attempt in 0..4 {
            self.anilist_rl.wait().await;
            let resp = self
                .http
                .post("https://graphql.anilist.co")
                .header("Accept", "application/json")
                .json(&body)
                .send()
                .await?;

            if let Some(limit) = resp
                .headers()
                .get("x-ratelimit-limit")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
            {
                if limit > 0 {
                    // Stay slightly under the advertised per-minute limit.
                    self.anilist_rl.set_interval(Duration::from_millis(60_000 / limit + 150));
                }
            }

            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let retry = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(30 * (attempt + 1));
                tokio::time::sleep(Duration::from_secs(retry.min(90))).await;
                continue;
            }

            let status = resp.status();
            let v: Value = resp.json().await?;
            if let Some(errors) = v.get("errors").and_then(|e| e.as_array()) {
                if !errors.is_empty() && v.get("data").map_or(true, |d| d.is_null()) {
                    bail!("AniList error ({status}): {}", errors[0].get("message").and_then(|m| m.as_str()).unwrap_or("unknown"));
                }
            }
            return Ok(v["data"].clone());
        }
        bail!("AniList rate limit exceeded")
    }

    pub async fn anilist_search(&self, search: &str, per_page: u32) -> Result<Vec<AlMedia>> {
        let q = format!(
            "query ($search: String, $perPage: Int) {{ Page(perPage: $perPage) {{ media(search: $search, type: ANIME, sort: SEARCH_MATCH) {{ {MEDIA_FIELDS} }} }} }}"
        );
        let data = self.anilist_query(&q, json!({ "search": search, "perPage": per_page })).await?;
        let list = data["Page"]["media"].clone();
        Ok(serde_json::from_value(list)?)
    }

    pub async fn anilist_media(&self, id: i64) -> Result<AlMedia> {
        let q = format!("query ($id: Int) {{ Media(id: $id, type: ANIME) {{ {MEDIA_FIELDS} }} }}");
        let data = self.anilist_query(&q, json!({ "id": id })).await?;
        if data["Media"].is_null() {
            bail!("AniList media {id} not found");
        }
        Ok(serde_json::from_value(data["Media"].clone())?)
    }
}

// ---------------------------------------------------------------------------
// Episode metadata (normalised)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct EpisodeMeta {
    pub ep_key: String,
    pub number: i64,
    pub is_special: bool,
    pub title_en: Option<String>,
    pub title_ja: Option<String>,
    pub title_romaji: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub runtime: Option<i64>,
    pub thumb_url: Option<String>,
    pub tvdb_season: Option<i64>,
    pub tvdb_episode: Option<i64>,
    pub absolute_number: Option<i64>,
    pub filler: bool,
    pub recap: bool,
}

fn non_empty(v: Option<&Value>) -> Option<String> {
    v.and_then(|x| x.as_str())
        .map(|s| s.trim().replace('`', "'"))
        .filter(|s| !s.is_empty())
}

/// Generic placeholder titles like "Episode 5" / "Episode S21" are not useful.
fn useful_title(t: Option<String>) -> Option<String> {
    t.filter(|s| {
        let l = s.to_ascii_lowercase();
        !(l.starts_with("episode ") && l[8..].trim_start_matches('s').chars().all(|c| c.is_ascii_digit()))
    })
}

impl Providers {
    /// Episodes from ani.zip, keyed to the AniList entry's own numbering.
    pub async fn anizip_episodes(&self, anilist_id: i64) -> Result<Vec<EpisodeMeta>> {
        self.anizip_rl.wait().await;
        let url = format!("https://api.ani.zip/mappings?anilist_id={anilist_id}");
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            bail!("ani.zip returned {}", resp.status());
        }
        let v: Value = resp.json().await?;
        let eps = v.get("episodes").and_then(|e| e.as_object()).ok_or_else(|| anyhow!("ani.zip: no episodes"))?;

        let mut out = Vec::new();
        for (key, e) in eps {
            let is_special = key.starts_with('S');
            let number: i64 = key.trim_start_matches('S').parse().unwrap_or(0);
            if number <= 0 {
                continue;
            }
            let title = e.get("title");
            let overview = non_empty(e.get("overview")).or_else(|| {
                non_empty(e.get("summary")).map(|s| {
                    // Strip trailing "Source: ..." attribution lines.
                    s.lines().filter(|l| !l.trim_start().starts_with("Source:")).collect::<Vec<_>>().join("\n").trim().to_string()
                })
            });
            out.push(EpisodeMeta {
                ep_key: key.clone(),
                number,
                is_special,
                title_en: useful_title(non_empty(title.and_then(|t| t.get("en")))),
                title_ja: non_empty(title.and_then(|t| t.get("ja"))),
                title_romaji: non_empty(title.and_then(|t| t.get("x-jat"))),
                overview: overview.filter(|s| !s.is_empty()),
                air_date: non_empty(e.get("airDate")).or_else(|| non_empty(e.get("airdate"))),
                runtime: e.get("runtime").and_then(|x| x.as_i64()).or_else(|| e.get("length").and_then(|x| x.as_i64())),
                thumb_url: non_empty(e.get("image")),
                tvdb_season: e.get("seasonNumber").and_then(|x| x.as_i64()),
                tvdb_episode: e.get("episodeNumber").and_then(|x| x.as_i64()),
                absolute_number: e.get("absoluteEpisodeNumber").and_then(|x| x.as_i64()),
                filler: false,
                recap: false,
            });
        }
        Ok(out)
    }

    /// Episode titles from Jikan (MyAnimeList). Used as a fallback.
    pub async fn jikan_episodes(&self, mal_id: i64) -> Result<Vec<EpisodeMeta>> {
        let mut out: BTreeMap<i64, EpisodeMeta> = BTreeMap::new();
        let mut page = 1;
        loop {
            self.jikan_rl.wait().await;
            let url = format!("https://api.jikan.moe/v4/anime/{mal_id}/episodes?page={page}");
            let resp = self.http.get(&url).send().await?;
            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                tokio::time::sleep(Duration::from_secs(3)).await;
                continue;
            }
            if !resp.status().is_success() {
                bail!("Jikan returned {}", resp.status());
            }
            let v: Value = resp.json().await?;
            let data = v["data"].as_array().cloned().unwrap_or_default();
            for e in data {
                let n = e["mal_id"].as_i64().unwrap_or(0);
                if n <= 0 {
                    continue;
                }
                out.insert(
                    n,
                    EpisodeMeta {
                        ep_key: n.to_string(),
                        number: n,
                        title_en: useful_title(non_empty(e.get("title"))),
                        title_ja: non_empty(e.get("title_japanese")),
                        title_romaji: non_empty(e.get("title_romanji")),
                        air_date: non_empty(e.get("aired")).map(|s| s.chars().take(10).collect()),
                        filler: e["filler"].as_bool().unwrap_or(false),
                        recap: e["recap"].as_bool().unwrap_or(false),
                        ..Default::default()
                    },
                );
            }
            let has_next = v["pagination"]["has_next_page"].as_bool().unwrap_or(false);
            if !has_next || page >= 30 {
                break;
            }
            page += 1;
        }
        Ok(out.into_values().collect())
    }
}
