//! BitTorrent Message Stream Encryption / Protocol Encryption (MSE/PE):
//! - Pure-Rust 768-bit Diffie-Hellman key exchange ($G = 2$, standard 768-bit MSE prime $P$).
//! - Pure-Rust RC4-drop1024 stream cipher.
//! - `PeerStream` transport wrapper supporting both plaintext TCP and RC4-encrypted TCP,
//!   with automatic plaintext -> MSE/PE fallback so seeders configured with *"Require encryption"*
//!   accept Kura's connections seamlessly.

use crate::torrent::sha1_digest;
use anyhow::{bail, Result};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Standard 768-bit safe prime $P$ specified by BitTorrent MSE/PE (big-endian, 96 bytes).
pub const MSE_PRIME_BE: [u8; 96] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xC9, 0x0F, 0xDA, 0xA2, 0x21, 0x68, 0xC2, 0x34,
    0xC4, 0xC6, 0x62, 0x8B, 0x80, 0xDC, 0x1C, 0xD1, 0x29, 0x02, 0x4E, 0x08, 0x8A, 0x67, 0xCC, 0x74,
    0x02, 0x0B, 0xBE, 0xA6, 0x3B, 0x13, 0x9B, 0x22, 0x51, 0x4A, 0x08, 0x79, 0x8E, 0x34, 0x04, 0xDD,
    0xEF, 0x95, 0x19, 0xB3, 0xCD, 0x3A, 0x43, 0x1B, 0x30, 0x2B, 0x0A, 0x6D, 0xF2, 0x5F, 0x14, 0x37,
    0x4F, 0xE1, 0x35, 0x6D, 0x6D, 0x51, 0xC2, 0x45, 0xE4, 0x85, 0xB5, 0x76, 0x62, 0x5E, 0x7E, 0xC6,
    0xF4, 0x4C, 0x42, 0xE9, 0xA6, 0x3A, 0x36, 0x21, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09, 0x05, 0x63,
];

const CRYPTO_PLAINTEXT: u32 = 0x01;
const CRYPTO_RC4: u32 = 0x02;

// ---------------------------------------------------------------------------
// 768-bit Modular Arithmetic (`[u64; 13]` little-endian limbs with overflow headroom)
// ---------------------------------------------------------------------------

type U832 = [u64; 13];

fn be96_to_limbs(bytes: &[u8; 96]) -> U832 {
    let mut out = [0u64; 13];
    for i in 0..12 {
        let off = 96 - (i + 1) * 8;
        out[i] = u64::from_be_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
            bytes[off + 4],
            bytes[off + 5],
            bytes[off + 6],
            bytes[off + 7],
        ]);
    }
    out
}

fn limbs_to_be96(limbs: &U832) -> [u8; 96] {
    let mut out = [0u8; 96];
    for i in 0..12 {
        let off = 96 - (i + 1) * 8;
        out[off..off + 8].copy_from_slice(&limbs[i].to_be_bytes());
    }
    out
}

#[inline]
fn ge_u832(a: &U832, b: &U832) -> bool {
    for i in (0..13).rev() {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

#[inline]
fn sub_assign_u832(a: &mut U832, b: &U832) {
    let mut borrow = 0u64;
    for i in 0..13 {
        let (d1, o1) = a[i].overflowing_sub(b[i]);
        let (d2, o2) = d1.overflowing_sub(borrow);
        a[i] = d2;
        borrow = (o1 as u64) + (o2 as u64);
    }
}

#[inline]
fn add_mod_u832(a: &U832, b: &U832, m: &U832) -> U832 {
    let mut out = [0u64; 13];
    let mut carry = 0u64;
    for i in 0..13 {
        let (s1, o1) = a[i].overflowing_add(b[i]);
        let (s2, o2) = s1.overflowing_add(carry);
        out[i] = s2;
        carry = (o1 as u64) + (o2 as u64);
    }
    if ge_u832(&out, m) {
        sub_assign_u832(&mut out, m);
    }
    out
}

#[inline]
fn shl1_mod_u832(a: &U832, m: &U832) -> U832 {
    let mut out = [0u64; 13];
    let mut carry = 0u64;
    for i in 0..13 {
        let next_carry = a[i] >> 63;
        out[i] = (a[i] << 1) | carry;
        carry = next_carry;
    }
    if ge_u832(&out, m) {
        sub_assign_u832(&mut out, m);
    }
    out
}

fn mul_mod_u832(a: &U832, b: &U832, m: &U832) -> U832 {
    let mut acc = [0u64; 13];
    let mut a_red = *a;
    while ge_u832(&a_red, m) {
        sub_assign_u832(&mut a_red, m);
    }
    for limb_idx in (0..12).rev() {
        let word = b[limb_idx];
        for bit in (0..64).rev() {
            acc = shl1_mod_u832(&acc, m);
            if ((word >> bit) & 1) != 0 {
                acc = add_mod_u832(&acc, &a_red, m);
            }
        }
    }
    acc
}

/// Compute `base ^ exp mod MSE_PRIME` where `base` is 96 bytes big-endian and `exp` is big-endian.
pub fn mod_pow_mse(base_be: &[u8; 96], exp_be: &[u8]) -> [u8; 96] {
    let m = be96_to_limbs(&MSE_PRIME_BE);
    let mut base = be96_to_limbs(base_be);
    while ge_u832(&base, &m) {
        sub_assign_u832(&mut base, &m);
    }
    let mut result = [0u64; 13];
    result[0] = 1;

    for &byte in exp_be {
        for bit in (0..8).rev() {
            result = mul_mod_u832(&result, &result, &m);
            if ((byte >> bit) & 1) != 0 {
                result = mul_mod_u832(&result, &base, &m);
            }
        }
    }
    limbs_to_be96(&result)
}

pub fn dh_public_key(priv_key: &[u8; 20]) -> [u8; 96] {
    let mut g = [0u8; 96];
    g[95] = 2;
    mod_pow_mse(&g, priv_key)
}

fn random_20_bytes(extra_seed: u64) -> [u8; 20] {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut material = Vec::with_capacity(40);
    material.extend_from_slice(&nanos.to_le_bytes());
    material.extend_from_slice(&seq.to_le_bytes());
    material.extend_from_slice(&extra_seed.to_le_bytes());
    material.extend_from_slice(&std::process::id().to_le_bytes());
    sha1_digest(&material)
}

// ---------------------------------------------------------------------------
// RC4-drop1024 Stream Cipher
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Rc4Cipher {
    s: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4Cipher {
    pub fn new_raw(key: &[u8]) -> Self {
        let mut s = [0u8; 256];
        for (idx, slot) in s.iter_mut().enumerate() {
            *slot = idx as u8;
        }
        let mut j: u8 = 0;
        for i in 0..256 {
            j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
            s.swap(i, j as usize);
        }
        Self { s, i: 0, j: 0 }
    }

    pub fn new_drop1024(key: &[u8]) -> Self {
        let mut c = Self::new_raw(key);
        let mut discard = [0u8; 1024];
        c.apply(&mut discard);
        c
    }

    #[inline]
    pub fn apply(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            self.i = self.i.wrapping_add(1);
            self.j = self.j.wrapping_add(self.s[self.i as usize]);
            self.s.swap(self.i as usize, self.j as usize);
            let k = self.s[(self.s[self.i as usize].wrapping_add(self.s[self.j as usize])) as usize];
            *byte ^= k;
        }
    }
}

fn mse_hash_key(prefix: &[u8; 4], secret: &[u8; 96], skey: &[u8; 20]) -> [u8; 20] {
    let mut buf = [0u8; 120];
    buf[0..4].copy_from_slice(prefix);
    buf[4..100].copy_from_slice(secret);
    buf[100..120].copy_from_slice(skey);
    sha1_digest(&buf)
}

fn mse_hash_req1(secret: &[u8; 96]) -> [u8; 20] {
    let mut buf = [0u8; 100];
    buf[0..4].copy_from_slice(b"req1");
    buf[4..100].copy_from_slice(secret);
    sha1_digest(&buf)
}

fn mse_hash_req2_xor_req3(skey: &[u8; 20], secret: &[u8; 96]) -> [u8; 20] {
    let mut b2 = [0u8; 24];
    b2[0..4].copy_from_slice(b"req2");
    b2[4..24].copy_from_slice(skey);
    let h2 = sha1_digest(&b2);

    let mut b3 = [0u8; 100];
    b3[0..4].copy_from_slice(b"req3");
    b3[4..100].copy_from_slice(secret);
    let h3 = sha1_digest(&b3);

    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = h2[i] ^ h3[i];
    }
    out
}

// ---------------------------------------------------------------------------
// Unified PeerStream (Plaintext TCP or MSE/PE RC4 TCP)
// ---------------------------------------------------------------------------

pub enum PeerStream {
    Plain(TcpStream),
    Encrypted {
        stream: TcpStream,
        read_rc4: Rc4Cipher,
        write_rc4: Rc4Cipher,
        preread: Vec<u8>,
        preread_pos: usize,
    },
}

impl PeerStream {
    pub async fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        match self {
            PeerStream::Plain(s) => {
                s.read_exact(buf).await?;
                Ok(())
            }
            PeerStream::Encrypted {
                stream,
                read_rc4,
                preread,
                preread_pos,
                ..
            } => {
                let mut written = 0usize;
                if *preread_pos < preread.len() {
                    let avail = (preread.len() - *preread_pos).min(buf.len());
                    buf[..avail].copy_from_slice(&preread[*preread_pos..*preread_pos + avail]);
                    *preread_pos += avail;
                    if *preread_pos >= preread.len() {
                        preread.clear();
                        *preread_pos = 0;
                    }
                    written += avail;
                }
                if written < buf.len() {
                    let tail = &mut buf[written..];
                    stream.read_exact(tail).await?;
                    read_rc4.apply(tail);
                }
                Ok(())
            }
        }
    }

    pub async fn write_all(&mut self, data: &[u8]) -> Result<()> {
        match self {
            PeerStream::Plain(s) => {
                s.write_all(data).await?;
                Ok(())
            }
            PeerStream::Encrypted {
                stream, write_rc4, ..
            } => {
                let mut enc = data.to_vec();
                write_rc4.apply(&mut enc);
                stream.write_all(&enc).await?;
                Ok(())
            }
        }
    }
}

/// Generic MSE/PE outbound handshake over any `AsyncRead + AsyncWrite + Unpin` stream
/// (returns `(read_rc4_opt, write_rc4_opt, preread_decrypted, peer_bt_handshake)`).
pub async fn perform_mse_outbound_io<S>(
    stream: &mut S,
    info_hash: &[u8; 20],
    bt_handshake: &[u8; 68],
) -> Result<(Option<(Rc4Cipher, Rc4Cipher, Vec<u8>)>, [u8; 68])>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let xa = random_20_bytes(u64::from_be_bytes([
        info_hash[0],
        info_hash[1],
        info_hash[2],
        info_hash[3],
        info_hash[4],
        info_hash[5],
        info_hash[6],
        info_hash[7],
    ]));
    let ya = dh_public_key(&xa);
    let pad_len = (xa[19] & 0x1F) as usize;
    let mut step1 = Vec::with_capacity(96 + pad_len);
    step1.extend_from_slice(&ya);
    step1.extend_from_slice(&xa[..(pad_len.min(20))]);
    if pad_len > 20 {
        step1.resize(96 + pad_len, 0xA5);
    }
    tokio::time::timeout(Duration::from_millis(3000), stream.write_all(&step1)).await??;

    let mut yb = [0u8; 96];
    tokio::time::timeout(Duration::from_millis(3500), stream.read_exact(&mut yb)).await??;
    let secret = mod_pow_mse(&yb, &xa);

    let key_a = mse_hash_key(b"keyA", &secret, info_hash);
    let key_b = mse_hash_key(b"keyB", &secret, info_hash);
    let mut write_rc4 = Rc4Cipher::new_drop1024(&key_a);
    let mut read_rc4 = Rc4Cipher::new_drop1024(&key_b);

    let mut expected_enc_vc = [0u8; 8];
    {
        let mut preview = read_rc4.clone();
        preview.apply(&mut expected_enc_vc);
    }

    let req1 = mse_hash_req1(&secret);
    let req23 = mse_hash_req2_xor_req3(info_hash, &secret);
    let mut enc_part = Vec::with_capacity(8 + 4 + 2 + 2 + 68);
    enc_part.extend_from_slice(&[0u8; 8]);
    enc_part.extend_from_slice(&(CRYPTO_RC4 | CRYPTO_PLAINTEXT).to_be_bytes());
    enc_part.extend_from_slice(&0u16.to_be_bytes());
    enc_part.extend_from_slice(&68u16.to_be_bytes());
    enc_part.extend_from_slice(bt_handshake);
    write_rc4.apply(&mut enc_part);

    let mut step3 = Vec::with_capacity(40 + enc_part.len());
    step3.extend_from_slice(&req1);
    step3.extend_from_slice(&req23);
    step3.extend_from_slice(&enc_part);
    tokio::time::timeout(Duration::from_millis(3000), stream.write_all(&step3)).await??;

    let mut scan_buf = Vec::with_capacity(256);
    let vc_pos = loop {
        if scan_buf.len() > 624 {
            bail!("MSE responder VC synchronization marker not found");
        }
        let mut tmp = [0u8; 64];
        let n = tokio::time::timeout(Duration::from_millis(3500), stream.read(&mut tmp)).await??;
        if n == 0 {
            bail!("EOF waiting for MSE VC");
        }
        scan_buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = scan_buf.windows(8).position(|w| w == expected_enc_vc) {
            break pos;
        }
    };

    let mut vc_consume = [0u8; 8];
    read_rc4.apply(&mut vc_consume);
    let mut tail = scan_buf[vc_pos + 8..].to_vec();
    if !tail.is_empty() {
        read_rc4.apply(&mut tail);
    }

    let mut tail_pos = 0usize;
    let read_dec = async |dst: &mut [u8],
                          s: &mut S,
                          rc4: &mut Rc4Cipher,
                          t: &Vec<u8>,
                          tp: &mut usize|
           -> Result<()> {
        let mut filled = 0usize;
        if *tp < t.len() {
            let take = (t.len() - *tp).min(dst.len());
            dst[..take].copy_from_slice(&t[*tp..*tp + take]);
            *tp += take;
            filled += take;
        }
        if filled < dst.len() {
            let rem = &mut dst[filled..];
            tokio::time::timeout(Duration::from_millis(3500), s.read_exact(rem)).await??;
            rc4.apply(rem);
        }
        Ok(())
    };

    let mut hdr = [0u8; 6];
    read_dec(&mut hdr, stream, &mut read_rc4, &tail, &mut tail_pos).await?;
    let crypto_select = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
    let pad_d_len = u16::from_be_bytes([hdr[4], hdr[5]]) as usize;
    if pad_d_len > 1024 {
        bail!("Invalid MSE PadD length: {pad_d_len}");
    }
    if pad_d_len > 0 {
        let mut pad_d = vec![0u8; pad_d_len];
        read_dec(&mut pad_d, stream, &mut read_rc4, &tail, &mut tail_pos).await?;
    }

    let mut peer_hs = [0u8; 68];
    if (crypto_select & CRYPTO_RC4) != 0 {
        read_dec(&mut peer_hs, stream, &mut read_rc4, &tail, &mut tail_pos).await?;
        let preread = if tail_pos < tail.len() {
            tail[tail_pos..].to_vec()
        } else {
            Vec::new()
        };
        Ok((Some((read_rc4, write_rc4, preread)), peer_hs))
    } else if (crypto_select & CRYPTO_PLAINTEXT) != 0 {
        if tail_pos < tail.len() {
            bail!("Ambiguous MSE plaintext boundary");
        }
        tokio::time::timeout(Duration::from_millis(3500), stream.read_exact(&mut peer_hs))
            .await??;
        Ok((None, peer_hs))
    } else {
        bail!("Unsupported MSE crypto_select: {crypto_select:#x}")
    }
}

/// Perform an outbound MSE/PE initiator handshake on `stream`, carrying our 68-byte BitTorrent
/// `bt_handshake` inside `IA` (Initial Payload) and returning `(PeerStream, peer_bt_handshake)`.
pub async fn perform_mse_outbound(
    mut stream: TcpStream,
    info_hash: &[u8; 20],
    bt_handshake: &[u8; 68],
) -> Result<(PeerStream, [u8; 68])> {
    let (cipher_opt, peer_hs) =
        perform_mse_outbound_io(&mut stream, info_hash, bt_handshake).await?;
    match cipher_opt {
        Some((read_rc4, write_rc4, preread)) => Ok((
            PeerStream::Encrypted {
                stream,
                read_rc4,
                write_rc4,
                preread,
                preread_pos: 0,
            },
            peer_hs,
        )),
        None => Ok((PeerStream::Plain(stream), peer_hs)),
    }
}

/// Connect to a BitTorrent peer: tries low-overhead plaintext handshake first, and if the peer
/// drops/rejects plaintext (e.g. qBittorrent/Deluge *"Require encryption"* mode), automatically
/// reconnects using MSE/PE RC4 encryption!
pub async fn connect_peer_with_mse_fallback(
    addr: SocketAddr,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
) -> Result<(PeerStream, [u8; 68])> {
    let mut hs = [0u8; 68];
    hs[0] = 19;
    hs[1..20].copy_from_slice(b"BitTorrent protocol");
    hs[25] |= 0x10; // BEP 10 Extension Protocol
    hs[28..48].copy_from_slice(&info_hash);
    hs[48..68].copy_from_slice(&peer_id);

    // 1. Try plaintext TCP handshake first
    let tcp = tokio::time::timeout(Duration::from_millis(2600), TcpStream::connect(addr)).await??;
    let _ = tcp.set_nodelay(true);
    let mut plain = tcp;

    let plain_attempt: Result<[u8; 68]> = async {
        tokio::time::timeout(Duration::from_millis(2500), plain.write_all(&hs)).await??;
        let mut resp_hs = [0u8; 68];
        tokio::time::timeout(Duration::from_millis(2800), plain.read_exact(&mut resp_hs))
            .await??;
        if &resp_hs[1..20] != b"BitTorrent protocol" || resp_hs[28..48] != info_hash {
            bail!("Plaintext handshake mismatch");
        }
        Ok(resp_hs)
    }
    .await;

    if let Ok(resp_hs) = plain_attempt {
        return Ok((PeerStream::Plain(plain), resp_hs));
    }
    drop(plain);

    // 2. TCP succeeded earlier, so host is reachable! Retry with MSE/PE RC4 handshake
    let mse_tcp =
        tokio::time::timeout(Duration::from_millis(2600), TcpStream::connect(addr)).await??;
    let _ = mse_tcp.set_nodelay(true);
    let (peer_stream, resp_hs) = perform_mse_outbound(mse_tcp, &info_hash, &hs).await?;
    if &resp_hs[1..20] != b"BitTorrent protocol" || resp_hs[28..48] != info_hash {
        bail!("MSE handshake BitTorrent header mismatch");
    }
    Ok((peer_stream, resp_hs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dh_768_shared_secret_commutes_and_rc4_roundtrips() {
        let xa = [0x11u8; 20];
        let xb = [0x27u8; 20];
        let ya = dh_public_key(&xa);
        let yb = dh_public_key(&xb);
        let s_ab = mod_pow_mse(&yb, &xa);
        let s_ba = mod_pow_mse(&ya, &xb);
        assert_eq!(s_ab, s_ba);
        assert_ne!(s_ab, [0u8; 96]);

        let key = b"mse-test-rc4-key";
        let mut enc = Rc4Cipher::new_drop1024(key);
        let mut dec = Rc4Cipher::new_drop1024(key);
        let orig = b"BitTorrent protocol encrypted payload verification!";
        let mut wire = orig.to_vec();
        enc.apply(&mut wire);
        assert_ne!(&wire[..], &orig[..]);
        dec.apply(&mut wire);
        assert_eq!(&wire[..], &orig[..]);
    }

    #[tokio::test]
    async fn mse_outbound_handshake_and_encrypted_stream_roundtrip() {
        let info_hash = sha1_digest(b"frieren-ep01-info");
        let initiator_peer_id = *b"-KR0081-111111111111";
        let responder_peer_id = *b"-qB4600-222222222222";

        let (mut client_io, mut server_io) = tokio::io::duplex(8192);

        let responder = tokio::spawn(async move {
            let mut ya_buf = [0u8; 96];
            server_io.read_exact(&mut ya_buf).await.unwrap();

            let xb = [0x42u8; 20];
            let yb = dh_public_key(&xb);
            let secret = mod_pow_mse(&ya_buf, &xb);
            server_io.write_all(&yb).await.unwrap();
            server_io.write_all(&[0x77u8; 12]).await.unwrap();

            let expected_req1 = mse_hash_req1(&secret);
            let mut rbuf = Vec::new();
            let req1_pos = loop {
                let mut chunk = [0u8; 64];
                let n = server_io.read(&mut chunk).await.unwrap();
                rbuf.extend_from_slice(&chunk[..n]);
                if let Some(p) = rbuf.windows(20).position(|w| w == expected_req1) {
                    break p;
                }
            };
            while rbuf.len() < req1_pos + 124 {
                let mut chunk = [0u8; 64];
                let n = server_io.read(&mut chunk).await.unwrap();
                rbuf.extend_from_slice(&chunk[..n]);
            }

            let key_a = mse_hash_key(b"keyA", &secret, &info_hash);
            let key_b = mse_hash_key(b"keyB", &secret, &info_hash);
            let mut r_from_init = Rc4Cipher::new_drop1024(&key_a);
            let mut w_to_init = Rc4Cipher::new_drop1024(&key_b);

            let mut enc_ia = rbuf[req1_pos + 40..req1_pos + 124].to_vec();
            r_from_init.apply(&mut enc_ia);
            assert_eq!(&enc_ia[0..8], &[0u8; 8]);
            let ia_len = u16::from_be_bytes([enc_ia[14], enc_ia[15]]) as usize;
            assert_eq!(ia_len, 68);
            assert_eq!(&enc_ia[16 + 1..16 + 20], b"BitTorrent protocol");

            let mut reply = Vec::new();
            reply.extend_from_slice(&[0u8; 8]);
            reply.extend_from_slice(&CRYPTO_RC4.to_be_bytes());
            reply.extend_from_slice(&4u16.to_be_bytes());
            reply.extend_from_slice(&[1, 2, 3, 4]);
            let mut bt_hs = [0u8; 68];
            bt_hs[0] = 19;
            bt_hs[1..20].copy_from_slice(b"BitTorrent protocol");
            bt_hs[25] = 0x10;
            bt_hs[28..48].copy_from_slice(&info_hash);
            bt_hs[48..68].copy_from_slice(&responder_peer_id);
            reply.extend_from_slice(&bt_hs);
            reply.extend_from_slice(b"MSE_OK");
            w_to_init.apply(&mut reply);
            server_io.write_all(&reply).await.unwrap();

            let mut rx_after = [0u8; 5];
            server_io.read_exact(&mut rx_after).await.unwrap();
            r_from_init.apply(&mut rx_after);
            assert_eq!(&rx_after, &[0, 0, 0, 1, 2]);
        });

        let mut init_hs = [0u8; 68];
        init_hs[0] = 19;
        init_hs[1..20].copy_from_slice(b"BitTorrent protocol");
        init_hs[25] = 0x10;
        init_hs[28..48].copy_from_slice(&info_hash);
        init_hs[48..68].copy_from_slice(&initiator_peer_id);

        let (cipher_opt, resp_hs) =
            perform_mse_outbound_io(&mut client_io, &info_hash, &init_hs)
                .await
                .unwrap();
        assert_eq!(&resp_hs[48..68], &responder_peer_id);
        let (mut read_rc4, mut write_rc4, preread) = cipher_opt.unwrap();
        let mut msg = preread;
        if msg.len() < 6 {
            let mut rem = vec![0u8; 6 - msg.len()];
            client_io.read_exact(&mut rem).await.unwrap();
            read_rc4.apply(&mut rem);
            msg.extend_from_slice(&rem);
        }
        assert_eq!(&msg, b"MSE_OK");

        let mut interested = [0, 0, 0, 1, 2];
        write_rc4.apply(&mut interested);
        client_io.write_all(&interested).await.unwrap();
        responder.await.unwrap();
    }
}
