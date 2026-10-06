//! Scoring AniList search candidates against a parsed title.

use crate::providers::AlMedia;

/// Lowercase, strip punctuation (unicode aware), collapse whitespace.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            last_space = false;
        } else if c == '\'' || c == '’' {
            // "Journey's" -> "journeys"
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

fn tokens(s: &str) -> Vec<&str> {
    s.split(' ').filter(|t| !t.is_empty()).collect()
}

/// Similarity in [0, 1] between a query title and a candidate title.
pub fn similarity(query: &str, candidate: &str) -> f64 {
    let q = normalize(query);
    let c = normalize(candidate);
    if q.is_empty() || c.is_empty() {
        return 0.0;
    }
    if q == c {
        return 1.0;
    }
    // Ignore spacing differences ("Re:Zero" vs "Re Zero", "Kaguya-sama" vs "Kaguyasama").
    let qs: String = q.chars().filter(|c| !c.is_whitespace()).collect();
    let cs: String = c.chars().filter(|c| !c.is_whitespace()).collect();
    if qs == cs {
        return 0.98;
    }

    let lev = strsim::normalized_levenshtein(&qs, &cs);
    let jw = strsim::jaro_winkler(&q, &c) * 0.92;

    // Token containment: all query tokens appear in the candidate ("Frieren" in "Sousou no Frieren").
    let qt = tokens(&q);
    let ct = tokens(&c);
    let found = qt.iter().filter(|t| ct.contains(t)).count();
    let containment = if found == qt.len() && !qt.is_empty() {
        let ratio = qt.len() as f64 / ct.len().max(1) as f64;
        // Single short tokens are weak evidence.
        let weight = if qt.len() == 1 && qs.chars().count() < 4 { 0.5 } else { 1.0 };
        (0.78 + 0.2 * ratio) * weight
    } else {
        0.0
    };

    // Prefix match: candidate starts with the query ("Mushoku Tensei" vs "Mushoku Tensei: Isekai Ittara Honki Dasu").
    let prefix = if cs.starts_with(&qs) && qs.chars().count() >= 5 { 0.9 } else { 0.0 };

    lev.max(jw).max(containment).max(prefix).min(1.0)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MatchHints {
    pub year: Option<u32>,
    /// The group looks like a movie (single file, no episode numbers).
    pub movie: bool,
    /// The group looks like an OVA/special collection.
    pub special: bool,
}

/// Score one candidate. Returns a confidence in [0, 1].
pub fn score_candidate(query: &str, alt_query: Option<&str>, m: &AlMedia, rank: usize, hints: MatchHints) -> f64 {
    let mut best: f64 = 0.0;
    for t in m.all_titles() {
        best = best.max(similarity(query, &t));
        if let Some(alt) = alt_query {
            best = best.max(similarity(alt, &t) * 0.97);
        }
    }
    let mut score = best;

    // AniList's own relevance ranking is a useful tie-breaker.
    score += 0.04 * (1.0 - (rank.min(8) as f64 / 8.0));

    let format = m.format.as_deref().unwrap_or("");
    if hints.movie {
        if format == "MOVIE" {
            score += 0.08;
        } else if matches!(format, "TV" | "TV_SHORT") {
            score -= 0.04;
        }
    } else if hints.special {
        if matches!(format, "OVA" | "SPECIAL" | "ONA") {
            score += 0.05;
        }
    } else if matches!(format, "TV" | "TV_SHORT" | "ONA") {
        score += 0.04;
    }
    if matches!(format, "MUSIC") {
        score -= 0.3;
    }

    if let (Some(y), Some(my)) = (hints.year, m.season_year.or(m.start_date.as_ref().and_then(|d| d.year))) {
        let diff = (y as i64 - my).abs();
        if diff == 0 {
            score += 0.08;
        } else if diff == 1 {
            score += 0.03;
        } else {
            score -= 0.08;
        }
    }

    // Not clamped: small bonuses must still break ties between perfect title matches.
    score.max(0.0)
}

/// Pick the best candidate. Returns (index, confidence in [0, 1]).
pub fn best_candidate(query: &str, alt_query: Option<&str>, cands: &[AlMedia], hints: MatchHints) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for (i, m) in cands.iter().enumerate() {
        let s = score_candidate(query, alt_query, m, i, hints);
        // Strictly greater: earlier (more relevant) results win ties.
        if best.map_or(true, |(_, b)| s > b) {
            best = Some((i, s));
        }
    }
    best.map(|(i, s)| (i, s.min(1.0)))
}

/// Strict check for "these two titles refer to the same show" (used for folder vs file names).
pub fn titles_related(a: &str, b: &str) -> bool {
    let na = normalize(a);
    let nb = normalize(b);
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    let ta = tokens(&na);
    let tb = tokens(&nb);
    let contains = |x: &[&str], y: &[&str]| x.iter().all(|t| y.contains(t));
    if contains(&ta, &tb) || contains(&tb, &ta) {
        return true;
    }
    let sa: String = na.chars().filter(|c| !c.is_whitespace()).collect();
    let sb: String = nb.chars().filter(|c| !c.is_whitespace()).collect();
    strsim::normalized_levenshtein(&sa, &sb) >= 0.6
}

pub const ACCEPT_THRESHOLD: f64 = 0.6;
pub const REVIEW_THRESHOLD: f64 = 0.8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation() {
        assert_eq!(normalize("Re:Zero kara Hajimeru"), "re zero kara hajimeru");
        assert_eq!(normalize("Frieren: Beyond Journey's End"), "frieren beyond journeys end");
        assert_eq!(normalize("葬送のフリーレン"), "葬送のフリーレン");
    }

    #[test]
    fn similarity_cases() {
        assert!(similarity("Sousou no Frieren", "Sousou no Frieren") > 0.99);
        assert!(similarity("Frieren", "Sousou no Frieren") > 0.8);
        assert!(similarity("Frieren Beyond Journeys End", "Frieren: Beyond Journey’s End") > 0.95);
        assert!(similarity("Re Zero", "Re:Zero") > 0.95);
        assert!(similarity("Mushoku Tensei", "Mushoku Tensei: Isekai Ittara Honki Dasu") > 0.85);
        assert!(similarity("Bocchi the Rock!", "BOCCHI THE ROCK!") > 0.99);
        assert!(similarity("Frieren", "Naruto") < 0.5);
    }

    #[test]
    fn related_titles() {
        assert!(titles_related("Frieren", "Sousou no Frieren"));
        assert!(titles_related("Re Zero", "Re:Zero"));
        assert!(!titles_related("Seasonal", "Dandadan"));
        assert!(!titles_related("Downloads", "Kusuriya no Hitorigoto"));
    }
}
