//! Minimal Mainline DHT (BEP 5) client: iterative `get_peers` lookups only.
//!
//! Used to find peers (and, via BEP 9, metadata) for magnet-only releases when trackers are slow,
//! down, or simply don't know the swarm. Kura doesn't keep a persistent routing table or answer
//! queries; every lookup bootstraps from the well-known routers and walks towards the info_hash.

use crate::torrent::{parse_bencode, sha1_digest, BVal};
use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::UnboundedSender;

const BOOTSTRAP: [&str; 4] = [
    "router.bittorrent.com:6881",
    "dht.transmissionbt.com:6881",
    "router.utorrent.com:6881",
    "dht.libtorrent.org:25401",
];

const ALPHA: usize = 8;
const MAX_IN_FLIGHT: usize = 24;
const MAX_QUERIES: usize = 400;
const QUERY_TIMEOUT: Duration = Duration::from_millis(2000);

fn node_id() -> [u8; 20] {
    let seed = format!(
        "kura-dht-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    sha1_digest(seed.as_bytes())
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

fn parse_compact_peer(b: &[u8]) -> Option<SocketAddr> {
    if b.len() != 6 {
        return None;
    }
    let ip = Ipv4Addr::new(b[0], b[1], b[2], b[3]);
    let port = u16::from_be_bytes([b[4], b[5]]);
    if port == 0 || ip.is_unspecified() || ip.is_loopback() || ip.is_private() {
        return None;
    }
    Some(SocketAddr::V4(SocketAddrV4::new(ip, port)))
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

    // Candidate nodes keyed by address -> optional node id (bootstrap routers have none yet).
    let mut nodes: HashMap<SocketAddr, Option<[u8; 20]>> = HashMap::new();
    for host in BOOTSTRAP {
        if let Ok(Ok(addrs)) =
            tokio::time::timeout(Duration::from_secs(3), tokio::net::lookup_host(host)).await
        {
            for a in addrs.filter(|a| a.is_ipv4()).take(2) {
                nodes.insert(a, None);
            }
        }
    }
    if nodes.is_empty() {
        anyhow::bail!("DHT bootstrap failed");
    }

    let mut queried: HashSet<SocketAddr> = HashSet::new();
    let mut in_flight: HashMap<u16, (SocketAddr, Instant)> = HashMap::new();
    let mut sent_peers: HashSet<SocketAddr> = HashSet::new();
    let mut next_tid: u16 = 1;
    let mut buf = vec![0u8; 2048];

    loop {
        if Instant::now() >= deadline || tx.is_closed() {
            break;
        }
        if cancelled.as_ref().map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
            break;
        }

        // Expire unanswered queries.
        in_flight.retain(|_, (_, t)| t.elapsed() < QUERY_TIMEOUT);

        // Query the closest not-yet-queried nodes (unknown-id bootstrap nodes first).
        if in_flight.len() < MAX_IN_FLIGHT && queried.len() < MAX_QUERIES {
            let mut pending: Vec<(SocketAddr, [u8; 20])> = nodes
                .iter()
                .filter(|(a, _)| !queried.contains(*a))
                .map(|(a, id)| (*a, id.map(|i| xor_distance(&i, &info_hash)).unwrap_or([0u8; 20])))
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
            break; // lookup exhausted
        }

        let recv = tokio::time::timeout(Duration::from_millis(250), sock.recv_from(&mut buf)).await;
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
        if let Some(id) = r.dict_get(b"id").and_then(|v| v.as_bytes()).filter(|b| b.len() == 20) {
            let mut nid = [0u8; 20];
            nid.copy_from_slice(id);
            nodes.insert(from, Some(nid));
        }
        if let Some(BVal::List(values)) = r.dict_get(b"values") {
            for v in values {
                if let Some(peer) = v.as_bytes().and_then(parse_compact_peer) {
                    if sent_peers.insert(peer) && tx.send(peer).is_err() {
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
                    if nodes.len() < 2000 {
                        nodes.entry(addr).or_insert(Some(nid));
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_valid_get_peers_query() {
        let q = get_peers_query(&[1u8; 20], &[2u8; 20], 0x0102);
        let (v, used) = parse_bencode(&q).unwrap();
        assert_eq!(used, q.len());
        assert_eq!(v.dict_get(b"q").and_then(|x| x.as_str()).as_deref(), Some("get_peers"));
        let a = v.dict_get(b"a").unwrap();
        assert_eq!(a.dict_get(b"info_hash").and_then(|x| x.as_bytes()), Some(&[2u8; 20][..]));
        assert_eq!(v.dict_get(b"t").and_then(|x| x.as_bytes()), Some(&[1u8, 2u8][..]));
    }

    #[test]
    fn filters_unusable_compact_peers() {
        assert!(parse_compact_peer(&[8, 8, 8, 8, 0x1a, 0xe1]).is_some());
        assert!(parse_compact_peer(&[192, 168, 1, 2, 0x1a, 0xe1]).is_none());
        assert!(parse_compact_peer(&[8, 8, 8, 8, 0, 0]).is_none());
    }
}

