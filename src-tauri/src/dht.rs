//! Mainline DHT (BEP 5) client with a persistent warm in-memory routing table.
//!
//! Bootstraps once in the background after app startup (`dht::warm_up()`) and retains verified,
//! responsive routing nodes across lookups so subsequent magnet `get_peers` walks start within
//! a few XOR hops of `info_hash` (<150ms first-peer latency, zero DNS stall).

use crate::klog;
use crate::torrent::{parse_bencode, sha1_digest, BVal};
use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::UnboundedSender;

const BOOTSTRAP: [&str; 5] = [
    "router.bittorrent.com:6881",
    "dht.transmissionbt.com:6881",
    "router.utorrent.com:6881",
    "dht.libtorrent.org:25401",
    "dht.aelitis.com:6881",
];

const ALPHA: usize = 10;
const MAX_IN_FLIGHT: usize = 32;
const MAX_QUERIES: usize = 450;
const MAX_WARM_NODES: usize = 512;
const QUERY_TIMEOUT: Duration = Duration::from_millis(1800);

static WARM_NODES: OnceLock<Mutex<HashMap<SocketAddr, [u8; 20]>>> = OnceLock::new();
static BOOTSTRAP_ADDRS: OnceLock<Mutex<Vec<SocketAddr>>> = OnceLock::new();

fn warm_nodes() -> &'static Mutex<HashMap<SocketAddr, [u8; 20]>> {
    WARM_NODES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn bootstrap_addrs_cache() -> &'static Mutex<Vec<SocketAddr>> {
    BOOTSTRAP_ADDRS.get_or_init(|| Mutex::new(Vec::new()))
}

fn node_id() -> [u8; 20] {
    static OWN_ID: OnceLock<[u8; 20]> = OnceLock::new();
    *OWN_ID.get_or_init(crate::db::random_bytes::<20>)
}

fn xor_distance(a: &[u8; 20], b: &[u8; 20]) -> [u8; 20] {
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = a[i] ^ b[i];
    }
    out
}

fn get_peers_query(own_id: &[u8; 20], info_hash: &[u8; 20], tid: u16) -> Vec<u8> {
    let mut q = Vec::with_capacity(96);
    q.extend_from_slice(b"d1:ad2:id20:");
    q.extend_from_slice(own_id);
    q.extend_from_slice(b"9:info_hash20:");
    q.extend_from_slice(info_hash);
    q.extend_from_slice(b"e1:q9:get_peers1:t2:");
    q.extend_from_slice(&tid.to_be_bytes());
    q.extend_from_slice(b"1:y1:qe");
    q
}

fn find_node_query(own_id: &[u8; 20], target: &[u8; 20], tid: u16) -> Vec<u8> {
    let mut q = Vec::with_capacity(96);
    q.extend_from_slice(b"d1:ad2:id20:");
    q.extend_from_slice(own_id);
    q.extend_from_slice(b"6:target20:");
    q.extend_from_slice(target);
    q.extend_from_slice(b"e1:q9:find_node1:t2:");
    q.extend_from_slice(&tid.to_be_bytes());
    q.extend_from_slice(b"1:y1:qe");
    q
}

fn parse_compact_peer(b: &[u8]) -> Option<SocketAddr> {
    if b.len() != 6 {
        return None;
    }
    let ip = Ipv4Addr::new(b[0], b[1], b[2], b[3]);
    let port = u16::from_be_bytes([b[4], b[5]]);
    if port == 0
        || ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
    {
        return None;
    }
    Some(SocketAddr::V4(SocketAddrV4::new(ip, port)))
}

async fn resolve_bootstrap_addrs() -> Vec<SocketAddr> {
    {
        let g = bootstrap_addrs_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !g.is_empty() {
            return g.clone();
        }
    }

    let mut handles = Vec::new();
    for host in BOOTSTRAP {
        handles.push(tokio::spawn(async move {
            let mut found = Vec::new();
            if let Ok(Ok(addrs)) =
                tokio::time::timeout(Duration::from_secs(2), tokio::net::lookup_host(host)).await
            {
                for a in addrs.filter(|a| a.is_ipv4()).take(2) {
                    found.push(a);
                }
            }
            found
        }));
    }

    let mut resolved = Vec::new();
    for h in handles {
        if let Ok(addrs) = h.await {
            for a in addrs {
                if !resolved.contains(&a) {
                    resolved.push(a);
                }
            }
        }
    }

    if !resolved.is_empty() {
        let mut g = bootstrap_addrs_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *g = resolved.clone();
    }
    resolved
}

fn merge_verified_nodes(verified: &HashMap<SocketAddr, [u8; 20]>) {
    if verified.is_empty() {
        return;
    }
    let mut table = warm_nodes().lock().unwrap_or_else(|e| e.into_inner());
    for (addr, id) in verified {
        table.insert(*addr, *id);
    }
    if table.len() > MAX_WARM_NODES {
        let excess = table.len() - MAX_WARM_NODES;
        let to_drop: Vec<SocketAddr> = table
            .keys()
            .filter(|a| !verified.contains_key(*a))
            .take(excess)
            .copied()
            .collect();
        for k in to_drop {
            table.remove(&k);
        }
    }
}

/// Seed the warm DHT routing table in the background after app startup so magnet streams
/// discover peers immediately without cold DNS + router bootstrap latency.
pub fn warm_up() {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1400)).await;
        let boot = resolve_bootstrap_addrs().await;
        if boot.is_empty() {
            return;
        }
        let Ok(sock) = UdpSocket::bind("0.0.0.0:0").await else {
            return;
        };
        let own_id = node_id();
        let targets: [[u8; 20]; 4] = [
            own_id,
            sha1_digest(b"kura-dht-bucket-01"),
            sha1_digest(b"kura-dht-bucket-02"),
            sha1_digest(b"kura-dht-bucket-03"),
        ];

        let mut tid: u16 = 1;
        for addr in &boot {
            for t in &targets {
                let pkt = find_node_query(&own_id, t, tid);
                tid = tid.wrapping_add(1).max(1);
                let _ = sock.send_to(&pkt, addr).await;
            }
        }

        let mut discovered: Vec<(SocketAddr, [u8; 20])> = Vec::new();
        let mut verified: HashMap<SocketAddr, [u8; 20]> = HashMap::new();
        let mut buf = vec![0u8; 2048];
        let phase1_end = Instant::now() + Duration::from_millis(1200);
        while Instant::now() < phase1_end {
            let Ok(Ok((n, from))) =
                tokio::time::timeout(Duration::from_millis(200), sock.recv_from(&mut buf)).await
            else {
                continue;
            };
            let Ok((msg, _)) = parse_bencode(&buf[..n]) else {
                continue;
            };
            let Some(r) = msg.dict_get(b"r") else {
                continue;
            };
            if let Some(id) = r
                .dict_get(b"id")
                .and_then(|v| v.as_bytes())
                .filter(|b| b.len() == 20)
            {
                let mut nid = [0u8; 20];
                nid.copy_from_slice(id);
                verified.insert(from, nid);
            }
            if let Some(compact) = r.dict_get(b"nodes").and_then(|v| v.as_bytes()) {
                for chunk in compact.chunks_exact(26) {
                    let mut nid = [0u8; 20];
                    nid.copy_from_slice(&chunk[0..20]);
                    if let Some(addr) = parse_compact_peer(&chunk[20..26]) {
                        if discovered.len() < 160 {
                            discovered.push((addr, nid));
                        }
                    }
                }
            }
        }

        for (idx, (addr, _)) in discovered.iter().take(96).enumerate() {
            let t = &targets[idx % targets.len()];
            let pkt = find_node_query(&own_id, t, tid);
            tid = tid.wrapping_add(1).max(1);
            let _ = sock.send_to(&pkt, addr).await;
        }

        let phase2_end = Instant::now() + Duration::from_millis(1400);
        while Instant::now() < phase2_end {
            let Ok(Ok((n, from))) =
                tokio::time::timeout(Duration::from_millis(200), sock.recv_from(&mut buf)).await
            else {
                continue;
            };
            let Ok((msg, _)) = parse_bencode(&buf[..n]) else {
                continue;
            };
            if let Some(r) = msg.dict_get(b"r") {
                if let Some(id) =
                    r.dict_get(b"id").and_then(|v| v.as_bytes()).filter(|b| b.len() == 20)
                {
                    let mut nid = [0u8; 20];
                    nid.copy_from_slice(id);
                    verified.insert(from, nid);
                }
            }
        }

        let count = verified.len();
        merge_verified_nodes(&verified);
        if count > 0 {
            klog!("[dht] Warmed routing table with {count} verified Mainline DHT nodes");
        }
    });
}

/// Run an iterative `get_peers` lookup for up to `duration`, streaming every discovered peer into `tx`.
/// Stops early when `cancelled` is set or `tx` is closed.
pub fn spawn_get_peers(
    info_hash: [u8; 20],
    tx: UnboundedSender<SocketAddr>,
    duration: Duration,
    cancelled: Option<Arc<AtomicBool>>,
) {
    tokio::spawn(async move {
        let _ = run_get_peers(info_hash, tx, duration, cancelled).await;
    });
}

async fn run_get_peers(
    info_hash: [u8; 20],
    tx: UnboundedSender<SocketAddr>,
    duration: Duration,
    cancelled: Option<Arc<AtomicBool>>,
) -> anyhow::Result<()> {
    let sock = UdpSocket::bind("0.0.0.0:0").await?;
    let own_id = node_id();
    let deadline = Instant::now() + duration;

    let mut nodes: HashMap<SocketAddr, Option<[u8; 20]>> = HashMap::new();
    {
        let table = warm_nodes().lock().unwrap_or_else(|e| e.into_inner());
        if !table.is_empty() {
            let mut sorted: Vec<(SocketAddr, [u8; 20], [u8; 20])> = table
                .iter()
                .map(|(a, id)| (*a, *id, xor_distance(id, &info_hash)))
                .collect();
            sorted.sort_by(|a, b| a.2.cmp(&b.2));
            for (addr, id, _) in sorted.into_iter().take(128) {
                nodes.insert(addr, Some(id));
            }
        }
    }

    for addr in resolve_bootstrap_addrs().await {
        nodes.entry(addr).or_insert(None);
    }
    if nodes.is_empty() {
        anyhow::bail!("DHT bootstrap failed");
    }

    let mut queried: HashSet<SocketAddr> = HashSet::new();
    let mut verified_responders: HashMap<SocketAddr, [u8; 20]> = HashMap::new();
    let mut in_flight: HashMap<u16, (SocketAddr, Instant)> = HashMap::new();
    let mut sent_peers: HashSet<SocketAddr> = HashSet::new();
    let mut next_tid: u16 = 1;
    let mut buf = vec![0u8; 2048];

    loop {
        if Instant::now() >= deadline || tx.is_closed() {
            break;
        }
        if cancelled
            .as_ref()
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(false)
        {
            break;
        }

        // Expire unanswered queries.
        in_flight.retain(|_, (_, t)| t.elapsed() < QUERY_TIMEOUT);

        // Query the closest not-yet-queried nodes.
        if in_flight.len() < MAX_IN_FLIGHT && queried.len() < MAX_QUERIES {
            let mut pending: Vec<(SocketAddr, [u8; 20])> = nodes
                .iter()
                .filter(|(a, _)| !queried.contains(*a))
                .map(|(a, id)| {
                    (
                        *a,
                        id.map(|i| xor_distance(&i, &info_hash))
                            .unwrap_or([0x04; 20]),
                    )
                })
                .collect();
            pending.sort_by(|a, b| a.1.cmp(&b.1));
            let budget = ALPHA.min(MAX_IN_FLIGHT - in_flight.len());
            for (addr, _) in pending.into_iter().take(budget) {
                let tid = next_tid;
                next_tid = next_tid.wrapping_add(1).max(1);
                let pkt = get_peers_query(&own_id, &info_hash, tid);
                if sock.send_to(&pkt, addr).await.is_ok() {
                    in_flight.insert(tid, (addr, Instant::now()));
                }
                queried.insert(addr);
            }
        }

        if in_flight.is_empty() && nodes.keys().all(|a| queried.contains(a)) {
            break;
        }

        let recv = tokio::time::timeout(Duration::from_millis(200), sock.recv_from(&mut buf)).await;
        let Ok(Ok((n, from))) = recv else {
            continue;
        };
        let Ok((msg, _)) = parse_bencode(&buf[..n]) else {
            continue;
        };
        let tid = msg
            .dict_get(b"t")
            .and_then(|t| t.as_bytes())
            .filter(|t| t.len() == 2)
            .map(|t| u16::from_be_bytes([t[0], t[1]]));
        if let Some(tid) = tid {
            in_flight.remove(&tid);
        }
        let Some(r) = msg.dict_get(b"r") else {
            continue;
        };
        if let Some(id) = r
            .dict_get(b"id")
            .and_then(|v| v.as_bytes())
            .filter(|b| b.len() == 20)
        {
            let mut nid = [0u8; 20];
            nid.copy_from_slice(id);
            nodes.insert(from, Some(nid));
            if verified_responders.len() < MAX_WARM_NODES {
                verified_responders.insert(from, nid);
            }
        }
        if let Some(BVal::List(values)) = r.dict_get(b"values") {
            for v in values {
                if let Some(peer) = v.as_bytes().and_then(parse_compact_peer) {
                    if sent_peers.insert(peer) && tx.send(peer).is_err() {
                        merge_verified_nodes(&verified_responders);
                        return Ok(());
                    }
                }
            }
        }
        if let Some(compact) = r.dict_get(b"nodes").and_then(|v| v.as_bytes()) {
            for chunk in compact.chunks_exact(26) {
                let mut nid = [0u8; 20];
                nid.copy_from_slice(&chunk[0..20]);
                if let Some(addr) = parse_compact_peer(&chunk[20..26]) {
                    if nodes.len() < 2500 {
                        nodes.entry(addr).or_insert(Some(nid));
                    }
                }
            }
        }
    }

    merge_verified_nodes(&verified_responders);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_valid_get_peers_and_find_node_queries() {
        let q = get_peers_query(&[1u8; 20], &[2u8; 20], 0x0102);
        let (v, used) = parse_bencode(&q).unwrap();
        assert_eq!(used, q.len());
        assert_eq!(
            v.dict_get(b"q").and_then(|x| x.as_str()).as_deref(),
            Some("get_peers")
        );
        let a = v.dict_get(b"a").unwrap();
        assert_eq!(
            a.dict_get(b"info_hash").and_then(|x| x.as_bytes()),
            Some(&[2u8; 20][..])
        );
        assert_eq!(
            v.dict_get(b"t").and_then(|x| x.as_bytes()),
            Some(&[1u8, 2u8][..])
        );

        let fn_q = find_node_query(&[3u8; 20], &[4u8; 20], 0x0304);
        let (fv, fused) = parse_bencode(&fn_q).unwrap();
        assert_eq!(fused, fn_q.len());
        assert_eq!(
            fv.dict_get(b"q").and_then(|x| x.as_str()).as_deref(),
            Some("find_node")
        );
    }

    #[test]
    fn filters_unusable_compact_peers_and_merges_warm_nodes() {
        let pub_peer = parse_compact_peer(&[8, 8, 8, 8, 0x1a, 0xe1]).unwrap();
        assert!(parse_compact_peer(&[192, 168, 1, 2, 0x1a, 0xe1]).is_none());
        assert!(parse_compact_peer(&[8, 8, 8, 8, 0, 0]).is_none());

        let mut map = HashMap::new();
        map.insert(pub_peer, [9u8; 20]);
        merge_verified_nodes(&map);
        let table = warm_nodes().lock().unwrap();
        assert_eq!(table.get(&pub_peer), Some(&[9u8; 20]));
    }
}
