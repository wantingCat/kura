//! Anime release filename parser.
//!
//! Turns names like `[SubsPlease] Sousou no Frieren - 05 (1080p) [ABCD1234].mkv`
//! into structured data (title, season, episode, special kind, ...).
//! Release groups, resolutions, codecs, CRCs etc. are stripped from the title.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "webm", "m4v", "mov", "wmv", "flv", "ts", "m2ts", "ogm", "mpg", "mpeg",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpecialKind {
    Ova,
    Ona,
    Special,
    Movie,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Parsed {
    /// Cleaned title without season / part markers.
    pub title: String,
    /// Title including season markers (useful as an alternative search query).
    pub title_full: String,
    pub season: Option<u32>,
    pub part: Option<u32>,
    pub episode: Option<u32>,
    pub episode_end: Option<u32>,
    /// e.g. episode "12.5" — usually a recap/special.
    pub episode_fraction: bool,
    pub special: Option<SpecialKind>,
    pub special_number: Option<u32>,
    /// Openings, endings, PVs, menus, ... — not real content.
    pub extra: bool,
    pub year: Option<u32>,
    pub group: Option<String>,
    pub version: Option<u32>,
}

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

re!(RE_GROUP, r"^\s*[\[【]([^\]】]+)[\]】]");
re!(RE_BRACKETS, r"[\[\(\{【（]([^\]\)\}】）]*)[\]\)\}】）]");
re!(RE_YEAR, r"^\s*((?:19|20)\d{2})\s*$");
re!(RE_EXTRA, r"(?i)\b(NC\s?OP|NC\s?ED|NCOP\d*|NCED\d*|creditless|PV\s?\d*|CM\s?\d*|menu\s?\d*|trailer|teaser|preview|sample|bonus|making)\b");
re!(RE_TECH, r"(?i)\b(\d{3,4}p|\d{3,4}x\d{3,4}|4k|uhd|x26[45]|h\s?26[45]|hevc|avc|av1|aac(\s?\d\s\d|\d\.\d)?|flac|opus|e?ac3|dts(-hd)?|ddp?\s?\d(\s\d)?|truehd|web(-?dl|-?rip)?|bd(rip)?|blu-?ray|dvd(rip)?|hdtv|tv(rip)?|10-?bits?|8-?bits?|hi10p?|hdr(10)?|dual[- ]audio|multi[- ]?subs?|eng(lish)?[- ]?(subs?|dub)|subbed|dubbed|uncensored|uncut|remux|batch|complete|crunchyroll|amzn|dsnp|hidive|b-global|v\d)\b");
re!(RE_CRC, r"(?i)\b[0-9a-f]{8}\b");
re!(RE_SCENE_GROUP, r"-([A-Za-z0-9]+)\s*$");

// Episode patterns, in priority order.
re!(RE_SXXEYY, r"(?i)\bS(\d{1,2})\s?E(\d{1,4})(?:v(\d))?(?:\s?-\s?E?(\d{1,4}))?\b");
re!(RE_NXNN, r"(?i)\b(\d{1,2})x(\d{1,3})\b");
re!(RE_S_DASH_EP, r"(?i)\bS(\d{1,2})\s+-\s+(\d{1,4})(?:v(\d))?\b");
re!(RE_DASH_EP, r"(?:^|\s)-\s+(\d{1,4})(?:\.(\d))?(?:v(\d))?(?:\s*-\s*(\d{1,4}))?(?:\s|$)");
re!(RE_WORD_EP, r"(?i)\b(?:episode|episodio|ep\.?|e)\s?(\d{1,4})(?:v(\d))?\b");
re!(RE_JP_EP, r"第\s?(\d{1,4})\s?[話话集]");
re!(RE_SPECIAL, r"(?i)\b(OVA|OAD|ONA|SP|specials?|movie|gekijouban|film)\s?(\d{1,3})?\b");
re!(RE_TRAILING_NUM, r"(?:^|\s)(\d{1,4})(?:v(\d))?\s*$");

// Season / part markers.
re!(RE_SEASON_S, r"(?i)\bS(\d{1,2})\b");
re!(RE_SEASON_WORD, r"(?i)\b(?:season|saison)\s*(\d{1,2})\b");
re!(RE_SEASON_ORD, r"(?i)\b(\d{1,2})(?:st|nd|rd|th)\s+season\b");
re!(RE_SEASON_NAMED, r"(?i)\b(second|third|fourth|fifth|sixth)\s+season\b");
re!(RE_PART, r"(?i)\b(?:part|cour)\s*(\d)\b");
re!(RE_JP_SEASON, r"第\s?(\d{1,2})\s?期");
re!(RE_SPACES, r"\s+");

pub fn is_video_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn strip_extension(name: &str) -> &str {
    if let Some(idx) = name.rfind('.') {
        let ext = &name[idx + 1..];
        if VIDEO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            return &name[..idx];
        }
    }
    name
}

fn named_season(word: &str) -> u32 {
    match word.to_ascii_lowercase().as_str() {
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "fifth" => 5,
        "sixth" => 6,
        _ => 1,
    }
}

fn clean_title(s: &str) -> String {
    let collapsed = RE_SPACES.replace_all(s, " ");
    collapsed
        .trim()
        .trim_matches(|c: char| matches!(c, '-' | '_' | '~' | ':' | '|' | ',' | '.' | '+') || c.is_whitespace())
        .trim()
        .to_string()
}

/// Detect a season number inside arbitrary text.
fn find_season(s: &str) -> Option<u32> {
    if let Some(c) = RE_SEASON_WORD.captures(s) {
        return c[1].parse().ok();
    }
    if let Some(c) = RE_SEASON_ORD.captures(s) {
        return c[1].parse().ok();
    }
    if let Some(c) = RE_SEASON_NAMED.captures(s) {
        return Some(named_season(&c[1]));
    }
    if let Some(c) = RE_JP_SEASON.captures(s) {
        return c[1].parse().ok();
    }
    if let Some(c) = RE_SEASON_S.captures(s) {
        return c[1].parse().ok();
    }
    None
}

fn strip_season_markers(s: &str) -> String {
    let mut out = s.to_string();
    for re in [&*RE_SEASON_WORD, &*RE_SEASON_ORD, &*RE_SEASON_NAMED, &*RE_JP_SEASON, &*RE_SEASON_S, &*RE_PART] {
        out = re.replace_all(&out, " ").to_string();
    }
    clean_title(&out)
}

/// Shared normalisation: group, brackets, separators, tech tokens.
/// Returns the cleaned string with partially filled `Parsed`.
fn normalise(input: &str, p: &mut Parsed) -> String {
    let mut s = input.to_string();

    if let Some(c) = RE_GROUP.captures(&s) {
        p.group = Some(c[1].trim().to_string());
        s = s[c.get(0).unwrap().end()..].to_string();
    }

    // Inspect bracket contents for useful info, then drop them.
    for c in RE_BRACKETS.captures_iter(&s.clone()) {
        let inner = &c[1];
        if let Some(y) = RE_YEAR.captures(inner) {
            p.year = y[1].parse().ok();
        }
        if p.season.is_none() {
            if let Some(season) = find_season(inner) {
                p.season = Some(season);
            }
        }
        if RE_EXTRA.is_match(inner) {
            p.extra = true;
        }
    }
    s = RE_BRACKETS.replace_all(&s, " ").to_string();

    s = s.replace('_', " ");
    let had_spaces = s.trim().contains(' ');
    if !had_spaces {
        // Scene style: Title.S01E01.1080p.WEB.x264-GROUP
        if p.group.is_none() {
            if let Some(c) = RE_SCENE_GROUP.captures(&s) {
                if s.contains('.') {
                    p.group = Some(c[1].to_string());
                    s = s[..c.get(0).unwrap().start()].to_string();
                }
            }
        }
        s = s.replace('.', " ");
    }

    s = RE_TECH.replace_all(&s, " ").to_string();
    s = RE_CRC.replace_all(&s, |caps: &regex::Captures| {
        // Only drop hex blobs that contain at least one digit and one letter (CRC32-like).
        let m = &caps[0];
        if m.chars().any(|c| c.is_ascii_digit()) && m.chars().any(|c| c.is_ascii_alphabetic()) {
            " ".to_string()
        } else {
            m.to_string()
        }
    }).to_string();

    if RE_EXTRA.is_match(&s) {
        p.extra = true;
    }

    RE_SPACES.replace_all(&s, " ").trim().to_string()
}

fn finish_title(p: &mut Parsed, raw_title: &str) {
    let raw = clean_title(raw_title);
    if p.season.is_none() {
        p.season = find_season(&raw);
    }
    if let Some(c) = RE_PART.captures(&raw) {
        p.part = c[1].parse().ok();
    }
    // Year at the end of the title, e.g. "Hunter x Hunter 2011"
    let mut title = strip_season_markers(&raw);
    if p.year.is_none() {
        if let Some(c) = Regex::new(r"\s((?:19|20)\d{2})$").unwrap().captures(&title) {
            p.year = c[1].parse().ok();
            title = title[..c.get(0).unwrap().start()].trim().to_string();
        }
    }
    p.title_full = raw;
    p.title = title;
}

/// Parse a video file name (with or without extension).
pub fn parse_filename(name: &str) -> Parsed {
    let mut p = Parsed::default();
    let s = normalise(strip_extension(name), &mut p);

    let mut title_end: Option<usize> = None;

    if let Some(c) = RE_SXXEYY.captures(&s) {
        p.season = c[1].parse().ok();
        p.episode = c[2].parse().ok();
        p.version = c.get(3).and_then(|m| m.as_str().parse().ok());
        p.episode_end = c.get(4).and_then(|m| m.as_str().parse().ok());
        title_end = Some(c.get(0).unwrap().start());
    } else if let Some(c) = RE_NXNN.captures(&s) {
        p.season = c[1].parse().ok();
        p.episode = c[2].parse().ok();
        title_end = Some(c.get(0).unwrap().start());
    } else if let Some(c) = RE_S_DASH_EP.captures(&s) {
        p.season = c[1].parse().ok();
        p.episode = c[2].parse().ok();
        p.version = c.get(3).and_then(|m| m.as_str().parse().ok());
        title_end = Some(c.get(0).unwrap().start());
    } else if let Some(c) = RE_DASH_EP.captures(&s) {
        p.episode = c[1].parse().ok();
        p.episode_fraction = c.get(2).is_some();
        p.version = c.get(3).and_then(|m| m.as_str().parse().ok());
        p.episode_end = c.get(4).and_then(|m| m.as_str().parse().ok());
        title_end = Some(c.get(0).unwrap().start());
    } else if let Some(c) = RE_JP_EP.captures(&s) {
        p.episode = c[1].parse().ok();
        title_end = Some(c.get(0).unwrap().start());
    } else if let Some(c) = RE_WORD_EP.captures(&s) {
        p.episode = c[1].parse().ok();
        p.version = c.get(2).and_then(|m| m.as_str().parse().ok());
        title_end = Some(c.get(0).unwrap().start());
    }

    // Specials (OVA, SP, Movie...). Search only in the title part when an episode was found.
    let special_scope = &s[..title_end.unwrap_or(s.len())];
    let special_scope = if title_end.is_some() { special_scope } else { s.as_str() };
    if let Some(c) = RE_SPECIAL.captures(special_scope) {
        let kind = match c[1].to_ascii_lowercase().as_str() {
            "ova" | "oad" => SpecialKind::Ova,
            "ona" => SpecialKind::Ona,
            "movie" | "gekijouban" | "film" => SpecialKind::Movie,
            _ => SpecialKind::Special,
        };
        p.special = Some(kind);
        p.special_number = c.get(2).and_then(|m| m.as_str().parse().ok());
        let start = c.get(0).unwrap().start();
        // Title stops at the special marker ("Title OVA 2", "Title - SP1").
        title_end = Some(title_end.map_or(start, |e| e.min(start)));
        if p.special_number.is_none() && kind != SpecialKind::Movie {
            p.special_number = p.episode;
        }
    }

    if title_end.is_none() {
        if let Some(c) = RE_TRAILING_NUM.captures(&s) {
            let n: u32 = c[1].parse().unwrap_or(0);
            let digits = c[1].len();
            let before = &s[..c.get(0).unwrap().start()];
            let looks_like_year = digits == 4 && (1900..=2099).contains(&n);
            if (digits <= 3 || !before.trim().is_empty()) && !looks_like_year {
                p.episode = Some(n);
                p.version = c.get(2).and_then(|m| m.as_str().parse().ok());
                title_end = Some(c.get(0).unwrap().start());
            }
        }
    }

    let raw_title = &s[..title_end.unwrap_or(s.len())];
    finish_title(&mut p, raw_title);
    p
}

/// Parse a folder name (series / batch folder). Never infers episodes from trailing numbers.
pub fn parse_folder(name: &str) -> Parsed {
    let mut p = Parsed::default();
    let s = normalise(name, &mut p);
    let mut end = s.len();
    // Batch ranges like "Title - 01-12" or "Title (01-24)" (brackets already removed)
    if let Some(m) = Regex::new(r"\s-?\s*\d{1,4}\s?[-~]\s?\d{1,4}\s*$").unwrap().find(&s) {
        end = m.start();
    }
    if let Some(c) = RE_SPECIAL.captures(&s[..end]) {
        let kind = match c[1].to_ascii_lowercase().as_str() {
            "ova" | "oad" => SpecialKind::Ova,
            "ona" => SpecialKind::Ona,
            "movie" | "gekijouban" | "film" => SpecialKind::Movie,
            _ => SpecialKind::Special,
        };
        p.special = Some(kind);
    }
    finish_title(&mut p, &s[..end]);
    p
}

/// Folder names that hold specials / extras inside a series folder.
pub fn classify_subfolder(name: &str) -> Option<SubfolderKind> {
    let n = name.trim().to_ascii_lowercase();
    match n.as_str() {
        "specials" | "special" | "sp" | "sps" | "ova" | "ovas" | "oad" | "season 0" | "season 00" | "s0" | "s00" => {
            Some(SubfolderKind::Specials)
        }
        "extras" | "extra" | "nc" | "ncop" | "nced" | "creditless" | "bonus" | "menu" | "menus" | "pv" | "cm" | "scans"
        | "featurettes" | "trailers" | "samples" | "sample" | "subs" | "subtitles" | "fonts" | "attachments" => {
            Some(SubfolderKind::Extras)
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubfolderKind {
    Specials,
    Extras,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtraKind {
    Opening,
    Ending,
    Trailer,
    Pv,
    Bonus,
    Other,
}

re!(RE_NCOP_NUM, r"(?i)(?:nc\s?op|op)\s*(\d+)");
re!(RE_NCED_NUM, r"(?i)(?:nc\s?ed|ed)\s*(\d+)");
re!(RE_TRAILER_NUM, r"(?i)(?:trailer|teaser|preview)\s*(\d+)");
re!(RE_PV_NUM, r"(?i)(?:pv|cm)\s*(\d+)");

pub fn classify_extra(filename: &str) -> (ExtraKind, String) {
    let lower = filename.to_ascii_lowercase();
    if lower.contains("ncop")
        || lower.contains("nc op")
        || lower.contains("creditless op")
        || lower.contains("clean op")
        || lower.contains("opening")
    {
        let num = RE_NCOP_NUM.captures(filename).and_then(|c| c.get(1)).map(|m| m.as_str());
        let label = match num {
            Some(n) => format!("Opening {n} (Creditless)"),
            None => "Opening (Creditless)".to_string(),
        };
        (ExtraKind::Opening, label)
    } else if lower.contains("nced")
        || lower.contains("nc ed")
        || lower.contains("creditless ed")
        || lower.contains("clean ed")
        || lower.contains("ending")
    {
        let num = RE_NCED_NUM.captures(filename).and_then(|c| c.get(1)).map(|m| m.as_str());
        let label = match num {
            Some(n) => format!("Ending {n} (Creditless)"),
            None => "Ending (Creditless)".to_string(),
        };
        (ExtraKind::Ending, label)
    } else if lower.contains("trailer") || lower.contains("teaser") || lower.contains("preview") {
        let num = RE_TRAILER_NUM.captures(filename).and_then(|c| c.get(1)).map(|m| m.as_str());
        let label = match num {
            Some(n) => format!("Trailer {n}"),
            None => "Trailer".to_string(),
        };
        (ExtraKind::Trailer, label)
    } else if lower.contains("pv") || lower.contains("cm") {
        let num = RE_PV_NUM.captures(filename).and_then(|c| c.get(1)).map(|m| m.as_str());
        let label = match num {
            Some(n) => format!("Promo {n}"),
            None => "Promotional Video".to_string(),
        };
        (ExtraKind::Pv, label)
    } else if lower.contains("bonus")
        || lower.contains("making")
        || lower.contains("interview")
        || lower.contains("featurette")
    {
        (ExtraKind::Bonus, "Bonus Feature".to_string())
    } else {
        let stem = std::path::Path::new(filename)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| filename.to_string());
        (ExtraKind::Other, stem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(name: &str) -> (String, Option<u32>, Option<u32>) {
        let p = parse_filename(name);
        (p.title, p.season, p.episode)
    }

    #[test]
    fn subsplease_style() {
        assert_eq!(
            ep("[SubsPlease] Sousou no Frieren - 05 (1080p) [ABCD1234].mkv"),
            ("Sousou no Frieren".into(), None, Some(5))
        );
        let p = parse_filename("[SubsPlease] Sousou no Frieren - 05 (1080p) [ABCD1234].mkv");
        assert_eq!(p.group.as_deref(), Some("SubsPlease"));
    }

    #[test]
    fn sxxeyy_with_tech() {
        assert_eq!(
            ep("[SomeGroup] Mushoku Tensei S02E13 1080p HEVC.mkv"),
            ("Mushoku Tensei".into(), Some(2), Some(13))
        );
    }

    #[test]
    fn scene_style() {
        let p = parse_filename("Frieren.Beyond.Journeys.End.S01E03.1080p.WEB.H264-VARYG.mkv");
        assert_eq!(p.title, "Frieren Beyond Journeys End");
        assert_eq!(p.season, Some(1));
        assert_eq!(p.episode, Some(3));
        assert_eq!(p.group.as_deref(), Some("VARYG"));
    }

    #[test]
    fn version_suffix() {
        let p = parse_filename("[Erai-raws] Bocchi the Rock! - 07v2 [1080p][Multiple Subtitle].mkv");
        assert_eq!(p.title, "Bocchi the Rock!");
        assert_eq!(p.episode, Some(7));
        assert_eq!(p.version, Some(2));
    }

    #[test]
    fn simple_episode_word() {
        assert_eq!(ep("Episode 01.mkv"), ("".into(), None, Some(1)));
        assert_eq!(ep("Frieren Episode 12.mp4"), ("Frieren".into(), None, Some(12)));
        assert_eq!(ep("Frieren - Ep 12.mp4"), ("Frieren".into(), None, Some(12)));
    }

    #[test]
    fn trailing_number() {
        assert_eq!(ep("Bocchi the Rock 03.mkv"), ("Bocchi the Rock".into(), None, Some(3)));
        assert_eq!(ep("03.mkv"), ("".into(), None, Some(3)));
        assert_eq!(ep("Frieren 1999.mkv"), ("Frieren".into(), None, None));
    }

    #[test]
    fn season_in_title() {
        assert_eq!(
            ep("[Judas] Mushoku Tensei Season 2 - 05 [1080p][HEVC x265 10bit][Multi-Subs].mkv"),
            ("Mushoku Tensei".into(), Some(2), Some(5))
        );
        assert_eq!(
            ep("[SubsPlease] Kaguya-sama wa Kokurasetai S2 - 03 (1080p).mkv"),
            ("Kaguya-sama wa Kokurasetai".into(), Some(2), Some(3))
        );
        assert_eq!(
            ep("[Group] Shingeki no Kyojin 3rd Season - 10 [720p].mkv"),
            ("Shingeki no Kyojin".into(), Some(3), Some(10))
        );
        assert_eq!(
            ep("[Group] Overlord Second Season - 01.mkv"),
            ("Overlord".into(), Some(2), Some(1))
        );
    }

    #[test]
    fn part_marker() {
        let p = parse_filename("[Group] Mushoku Tensei Part 2 - 03 [1080p].mkv");
        assert_eq!(p.title, "Mushoku Tensei");
        assert_eq!(p.part, Some(2));
        assert_eq!(p.episode, Some(3));
        assert_eq!(p.title_full, "Mushoku Tensei Part 2");
    }

    #[test]
    fn titles_with_numbers() {
        assert_eq!(ep("[Group] 86 - 05 [1080p].mkv"), ("86".into(), None, Some(5)));
        assert_eq!(ep("[Group] Mob Psycho 100 - 11 [1080p].mkv"), ("Mob Psycho 100".into(), None, Some(11)));
        assert_eq!(ep("[Group] Steins;Gate 0 - 04 [BD 1080p].mkv"), ("Steins;Gate 0".into(), None, Some(4)));
    }

    #[test]
    fn year_in_brackets() {
        let p = parse_filename("[Group] Hunter x Hunter (2011) - 148 [1080p].mkv");
        assert_eq!(p.title, "Hunter x Hunter");
        assert_eq!(p.year, Some(2011));
        assert_eq!(p.episode, Some(148));
    }

    #[test]
    fn specials() {
        let p = parse_filename("[Group] Mushoku Tensei OVA [1080p].mkv");
        assert_eq!(p.title, "Mushoku Tensei");
        assert_eq!(p.special, Some(SpecialKind::Ova));

        let p = parse_filename("[Group] Bocchi the Rock! - SP1 [1080p].mkv");
        assert_eq!(p.title, "Bocchi the Rock!");
        assert_eq!(p.special, Some(SpecialKind::Special));
        assert_eq!(p.special_number, Some(1));

        let p = parse_filename("[Group] Re Zero OVA - 02 [1080p].mkv");
        assert_eq!(p.title, "Re Zero");
        assert_eq!(p.special, Some(SpecialKind::Ova));
        assert_eq!(p.special_number, Some(2));
    }

    #[test]
    fn movies() {
        let p = parse_filename("[Group] Kimi no Na wa. (2016) [BD 1080p HEVC FLAC].mkv");
        assert_eq!(p.title, "Kimi no Na wa");
        assert_eq!(p.year, Some(2016));
        assert_eq!(p.episode, None);

        let p = parse_filename("Suzume.2022.1080p.BluRay.x264-GRP.mkv");
        assert_eq!(p.title, "Suzume");
        assert_eq!(p.year, Some(2022));
        assert_eq!(p.episode, None);

        let p = parse_filename("[Group] Kimetsu no Yaiba Movie - Mugen Ressha-hen [1080p].mkv");
        assert_eq!(p.title, "Kimetsu no Yaiba");
        assert_eq!(p.special, Some(SpecialKind::Movie));
    }

    #[test]
    fn extras() {
        assert!(parse_filename("[Group] Frieren - NCOP1 [1080p].mkv").extra);
        assert!(parse_filename("[Group] Frieren - NCED [1080p].mkv").extra);
        assert!(parse_filename("[Group] Frieren [Menu01].mkv").extra);
        assert!(!parse_filename("[Group] Frieren - 01 [1080p].mkv").extra);
    }

    #[test]
    fn fractional_episode() {
        let p = parse_filename("[Group] Re Zero - 12.5 [1080p].mkv");
        assert_eq!(p.episode, Some(12));
        assert!(p.episode_fraction);
    }

    #[test]
    fn underscores() {
        assert_eq!(
            ep("[Group]_Toradora!_-_05_[720p][ABCDEF12].mkv"),
            ("Toradora!".into(), None, Some(5))
        );
    }

    #[test]
    fn japanese_episode() {
        assert_eq!(ep("葬送のフリーレン 第05話.mp4"), ("葬送のフリーレン".into(), None, Some(5)));
    }

    #[test]
    fn folders() {
        let p = parse_folder("[Judas] Mushoku Tensei S2 [1080p][HEVC x265 10bit][Batch]");
        assert_eq!(p.title, "Mushoku Tensei");
        assert_eq!(p.season, Some(2));

        let p = parse_folder("Mob Psycho 100");
        assert_eq!(p.title, "Mob Psycho 100");
        assert_eq!(p.episode, None);

        let p = parse_folder("Frieren");
        assert_eq!(p.title, "Frieren");

        let p = parse_folder("[Group] Bocchi the Rock! (01-12) [BD 1080p]");
        assert_eq!(p.title, "Bocchi the Rock!");

        let p = parse_folder("Re Zero - Season 2");
        assert_eq!(p.title, "Re Zero");
        assert_eq!(p.season, Some(2));

        let p = parse_folder("Hunter x Hunter (2011)");
        assert_eq!(p.title, "Hunter x Hunter");
        assert_eq!(p.year, Some(2011));
    }

    #[test]
    fn subfolders() {
        assert_eq!(classify_subfolder("Specials"), Some(SubfolderKind::Specials));
        assert_eq!(classify_subfolder("Extras"), Some(SubfolderKind::Extras));
        assert_eq!(classify_subfolder("Season 2"), None);
    }
}
