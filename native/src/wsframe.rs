//! Minimal RFC 6455 WebSocket: handshake helpers + frame codec.
//! Text frames only (the app protocol is JSON text), max 2 MB payload.

use std::io::{Read, Write};

/* ---------------- SHA-1 (handshake only) ---------------- */

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, item) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([item[0], item[1], item[2], item[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Sec-WebSocket-Accept value for a client key.
pub fn ws_accept(key: &str) -> String {
    let mut data = key.as_bytes().to_vec();
    data.extend_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64_encode(&sha1(&data))
}

/* ---------------- frames ---------------- */

#[derive(Debug, PartialEq)]
pub enum Frame {
    Text(String),
    Close,
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    /// Binary/unknown — ignored by the app but must be consumed.
    Other,
}

/// Read one frame. Returns None on clean EOF before any byte.
pub fn read_frame(stream: &mut impl Read) -> std::io::Result<Option<Frame>> {
    let mut head = [0u8; 2];
    match stream.read_exact(&mut head) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let opcode = head[0] & 0x0f;
    let masked = head[1] & 0x80 != 0;
    let mut len = (head[1] & 0x7f) as u64;
    if len == 126 {
        let mut b = [0u8; 2];
        stream.read_exact(&mut b)?;
        len = u16::from_be_bytes(b) as u64;
    } else if len == 127 {
        let mut b = [0u8; 8];
        stream.read_exact(&mut b)?;
        len = u64::from_be_bytes(b);
    }
    if len > 2 * 1024 * 1024 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut mask = [0u8; 4];
    if masked {
        stream.read_exact(&mut mask)?;
    }
    let mut payload = vec![0u8; len as usize];
    if len > 0 {
        stream.read_exact(&mut payload)?;
    }
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok(Some(match opcode {
        0x1 => Frame::Text(String::from_utf8_lossy(&payload).into_owned()),
        0x8 => Frame::Close,
        0x9 => Frame::Ping(payload),
        0xA => Frame::Pong(payload),
        _ => Frame::Other,
    }))
}

/// Write one frame (unmasked — server side; mask=true for client side).
pub fn write_frame(stream: &mut impl Write, frame: &Frame, mask: bool) -> std::io::Result<()> {
    let (opcode, payload): (u8, &[u8]) = match frame {
        Frame::Text(s) => (0x1, s.as_bytes()),
        Frame::Close => (0x8, &[]),
        Frame::Ping(p) => (0x9, p),
        Frame::Pong(p) => (0xA, p),
        Frame::Other => return Ok(()),
    };
    let mut head = Vec::with_capacity(10);
    head.push(0x80 | opcode);
    let len = payload.len();
    let mask_bit = if mask { 0x80 } else { 0 };
    if len < 126 {
        head.push(mask_bit | len as u8);
    } else if len <= 0xFFFF {
        head.push(mask_bit | 126);
        head.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        head.push(mask_bit | 127);
        head.extend_from_slice(&(len as u64).to_be_bytes());
    }
    if mask {
        // deterministic mask is fine for localhost traffic
        let key = [0x12u8, 0x34, 0x56, 0x78];
        head.extend_from_slice(&key);
        let mut masked = payload.to_vec();
        for (i, b) in masked.iter_mut().enumerate() {
            *b ^= key[i % 4];
        }
        stream.write_all(&head)?;
        stream.write_all(&masked)?;
    } else {
        stream.write_all(&head)?;
        stream.write_all(payload)?;
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_vectors() {
        assert_eq!(
            sha1(b"abc").iter().map(|b| format!("{:02x}", b)).collect::<String>(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            sha1(b"").iter().map(|b| format!("{:02x}", b)).collect::<String>(),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
    }

    #[test]
    fn ws_accept_vector() {
        // RFC 6455 §1.3 example
        assert_eq!(ws_accept("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn frame_roundtrip() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &Frame::Text("{\"a\":1}".into()), false).unwrap();
        let frame = read_frame(&mut buf.as_slice()).unwrap().unwrap();
        assert_eq!(frame, Frame::Text("{\"a\":1}".into()));
    }

    #[test]
    fn masked_roundtrip() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, &Frame::Ping(vec![1, 2, 3]), true).unwrap();
        let frame = read_frame(&mut buf.as_slice()).unwrap().unwrap();
        assert_eq!(frame, Frame::Ping(vec![1, 2, 3]));
    }
}
