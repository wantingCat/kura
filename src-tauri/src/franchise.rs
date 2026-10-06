//! Franchise grouping.
//!
//! AniList gives every season, cour, movie and OVA its own entry, so a show like
//! Attack on Titan becomes 6+ separate titles. Here we stitch owned entries back
//! together into franchises by walking PREQUEL / SEQUEL edges (plus OVA / special
//! side stories), so the UI can show one card per franchise with season tabs.
//!
//! Each entry keeps its own AniList id — grouping is purely a presentation layer.

use rusqlite::Connection;
use std::collections::HashMap;

/// Where an owned entry sits inside its franchise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// AniList id of the earliest owned entry; used as the franchise id.
    pub root: i64,
    /// 0-based position in release order among owned entries.
    pub index: usize,
    /// Number of owned entries in the franchise.
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct Owned {
    pub id: i64,
    pub format: Option<String>,
    /// `YYYY[-MM[-DD]]`, which sorts lexically.
    pub start: Option<String>,
    pub year: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub from: i64,
    pub to: i64,
    pub kind: String,
    pub to_format: Option<String>,
}

fn is_extra(format: Option<&str>) -> bool {
    matches!(format, Some("OVA" | "SPECIAL"))
}

/// Whether an edge should merge two entries into the same franchise.
fn links(from_format: Option<&str>, e: &Edge) -> bool {
    match e.kind.as_str() {
        "PREQUEL" | "SEQUEL" => true,
        // Side stories are often full spin-off series; only pull in short extras.
        "SIDE_STORY" => is_extra(e.to_format.as_deref()),
        "PARENT" => is_extra(from_format),
        _ => false,
    }
}

fn find(parent: &mut HashMap<i64, i64>, x: i64) -> i64 {
    parent.entry(x).or_insert(x);
    let mut root = x;
    while parent[&root] != root {
        root = parent[&root];
    }
    let mut cur = x;
    while parent[&cur] != root {
        let next = parent[&cur];
        parent.insert(cur, root);
        cur = next;
    }
    root
}

pub fn compute(owned: &[Owned], edges: &[Edge]) -> HashMap<i64, Placement> {
    let formats: HashMap<i64, Option<&str>> = owned.iter().map(|o| (o.id, o.format.as_deref())).collect();
    let mut parent: HashMap<i64, i64> = HashMap::new();
    for o in owned {
        find(&mut parent, o.id);
    }
    // Non-owned entries may appear as bridge nodes (own S1 + S3 but not S2), which
    // still connects S1 and S3 because both point at S2.
    for e in edges {
        let Some(from_format) = formats.get(&e.from) else { continue };
        if !links(*from_format, e) {
            continue;
        }
        let a = find(&mut parent, e.from);
        let b = find(&mut parent, e.to);
        if a != b {
            parent.insert(a, b);
        }
    }

    let mut components: HashMap<i64, Vec<&Owned>> = HashMap::new();
    for o in owned {
        let r = find(&mut parent, o.id);
        components.entry(r).or_default().push(o);
    }

    let mut out = HashMap::new();
    for (_, mut members) in components {
        members.sort_by(|a, b| {
            let ka = (a.start.as_deref().unwrap_or("9999"), a.year.unwrap_or(9999), a.id);
            let kb = (b.start.as_deref().unwrap_or("9999"), b.year.unwrap_or(9999), b.id);
            ka.cmp(&kb)
        });
        let root = members[0].id;
        let size = members.len();
        for (index, m) in members.iter().enumerate() {
            out.insert(m.id, Placement { root, index, size });
        }
    }
    out
}

/// Franchise placement for every entry that has at least one local file.
pub fn load(conn: &Connection) -> rusqlite::Result<HashMap<i64, Placement>> {
    let mut stmt = conn.prepare(
        "SELECT m.anilist_id, m.format, m.start_date, m.season_year FROM media m
         WHERE EXISTS (SELECT 1 FROM local_files f WHERE f.anilist_id = m.anilist_id)",
    )?;
    let owned = stmt
        .query_map([], |r| Ok(Owned { id: r.get(0)?, format: r.get(1)?, start: r.get(2)?, year: r.get(3)? }))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut stmt = conn.prepare(
        "SELECT r.anilist_id, r.related_id, r.relation_type, r.format FROM media_relations r
         WHERE COALESCE(r.media_type, 'ANIME') = 'ANIME'
           AND EXISTS (SELECT 1 FROM local_files f WHERE f.anilist_id = r.anilist_id)",
    )?;
    let edges = stmt
        .query_map([], |r| Ok(Edge { from: r.get(0)?, to: r.get(1)?, kind: r.get(2)?, to_format: r.get(3)? }))?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(compute(&owned, &edges))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(id: i64, format: &str, start: &str) -> Owned {
        Owned { id, format: Some(format.into()), start: Some(start.into()), year: start[..4].parse().ok() }
    }
    fn e(from: i64, to: i64, kind: &str, to_format: &str) -> Edge {
        Edge { from, to, kind: kind.into(), to_format: Some(to_format.into()) }
    }

    #[test]
    fn groups_sequel_chain_in_release_order() {
        // Inserted out of order on purpose.
        let owned = [o(3, "TV", "2019-04-29"), o(1, "TV", "2013-04-07"), o(2, "TV", "2017-04-01")];
        let edges = [e(1, 2, "SEQUEL", "TV"), e(2, 1, "PREQUEL", "TV"), e(2, 3, "SEQUEL", "TV")];
        let p = compute(&owned, &edges);
        assert_eq!(p[&1], Placement { root: 1, index: 0, size: 3 });
        assert_eq!(p[&2], Placement { root: 1, index: 1, size: 3 });
        assert_eq!(p[&3], Placement { root: 1, index: 2, size: 3 });
    }

    #[test]
    fn bridges_over_unowned_season() {
        // Own S1 and S3, not S2 (id 2).
        let owned = [o(1, "TV", "2013-04-07"), o(3, "TV", "2019-04-29")];
        let edges = [e(1, 2, "SEQUEL", "TV"), e(3, 2, "PREQUEL", "TV")];
        let p = compute(&owned, &edges);
        assert_eq!(p[&3].root, 1);
        assert_eq!(p[&3].size, 2);
    }

    #[test]
    fn ova_side_story_joins_but_spinoff_series_does_not() {
        let owned = [o(1, "TV", "2013-04-07"), o(5, "OVA", "2014-01-01"), o(9, "TV", "2020-01-01")];
        let edges = [e(1, 5, "SIDE_STORY", "OVA"), e(1, 9, "SIDE_STORY", "TV"), e(1, 9, "SPIN_OFF", "TV")];
        let p = compute(&owned, &edges);
        assert_eq!(p[&5].root, 1);
        assert_eq!(p[&9], Placement { root: 9, index: 0, size: 1 });
    }

    #[test]
    fn parent_edge_from_ova_joins() {
        let owned = [o(1, "TV", "2013-04-07"), o(5, "OVA", "2014-01-01")];
        let edges = [e(5, 1, "PARENT", "TV")];
        assert_eq!(compute(&owned, &edges)[&5].root, 1);
    }

    #[test]
    fn standalone_entry_is_its_own_franchise() {
        let owned = [o(7, "MOVIE", "2016-08-26")];
        let p = compute(&owned, &[]);
        assert_eq!(p[&7], Placement { root: 7, index: 0, size: 1 });
    }
}
