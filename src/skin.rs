//! Pure-Rust Mojang skin decoder (`UTF-16LE SHA-1` + `Base64` + `textures` JSON)
//! and deterministic 18-skin fallback selector.

use crate::constants::{DEFAULT_SKINS, DEFAULT_STEVE_WIDE};

#[derive(Clone)]
pub(crate) struct ResolvedSkin {
    pub(crate) primary: String,
    pub(crate) fallback: String,
    pub(crate) slim: bool,
}

impl Default for ResolvedSkin {
    fn default() -> Self {
        Self {
            primary: DEFAULT_STEVE_WIDE.into(),
            fallback: DEFAULT_STEVE_WIDE.into(),
            slim: false,
        }
    }
}

pub(crate) fn default_skin_for_uuid(uuid: u128) -> (&'static str, bool) {
    let most = (uuid >> 64) as u64;
    let least = uuid as u64;
    let xor = most ^ least;
    let folded = ((xor >> 32) as i32) ^ (xor as i32);
    let idx = folded.rem_euclid(18) as usize;
    DEFAULT_SKINS[idx]
}

pub(crate) fn sha1_hex_utf16le(text: &str) -> String {
    let mut msg = Vec::with_capacity(text.len() * 2 + 72);
    for unit in text.encode_utf16() {
        msg.extend_from_slice(&unit.to_le_bytes());
    }
    let bit_len = (msg.len() as u64) * 8;
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xEFCD_AB89;
    let mut h2: u32 = 0x98BA_DCFE;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xC3D2_E1F0;

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(40);
    for word in [h0, h1, h2, h3, h4] {
        for byte in word.to_be_bytes() {
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0F) as usize] as char);
        }
    }
    out
}

pub(crate) fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            break;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

/// Extracts `("skins/<sha1>", is_slim)` from Mojang's Base64 `"textures"` property JSON.
pub(crate) fn parse_mojang_skin_property(b64_value: &str) -> Option<(String, bool)> {
    let raw = base64_decode(b64_value)?;
    let json = core::str::from_utf8(&raw).ok()?;
    let skin_idx = json.find("\"SKIN\"")?;
    let skin_tail = &json[skin_idx..];
    let skin_section = match skin_tail.find("\"CAPE\"") {
        Some(cape_pos) => &skin_tail[..cape_pos],
        None => skin_tail,
    };

    let url_key = skin_section.find("\"url\"")?;
    let after_colon = &skin_section[url_key + 5..];
    let first_quote = after_colon.find('"')?;
    let rest_url = &after_colon[first_quote + 1..];
    let end_quote = rest_url.find('"')?;
    let url = &rest_url[..end_quote];
    let hash = url.rsplit('/').next()?.trim();
    if hash.is_empty() || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    let slim = if let Some(model_pos) = skin_section.find("\"model\"") {
        let model_tail = &skin_section[model_pos + 7..];
        model_tail
            .split('"')
            .nth(1)
            .is_some_and(|m| m.eq_ignore_ascii_case("slim"))
    } else {
        false
    };

    let sprite = format!("skins/{}", sha1_hex_utf16le(hash));
    Some((sprite, slim))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64_encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[((n >> 6) & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[(n & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    #[test]
    fn sha1_utf16le_matches_known_vectors() {
        assert_eq!(
            sha1_hex_utf16le(""),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        // UTF-16LE("abc") == [0x61, 0x00, 0x62, 0x00, 0x63, 0x00]
        assert_eq!(
            sha1_hex_utf16le("abc"),
            "9f04f41a848514162050e3d68c1a7abb441dc2b5"
        );
    }

    #[test]
    fn base64_decodes_standard_and_whitespace() {
        let payload = b"Just a Rifle 1873";
        let encoded = b64_encode(payload);
        assert_eq!(base64_decode(&encoded).as_deref(), Some(&payload[..]));
        let spaced = format!("  {}\n ", encoded);
        assert_eq!(base64_decode(&spaced).as_deref(), Some(&payload[..]));
        assert!(base64_decode("invalid$char").is_none());
    }

    #[test]
    fn mojang_skin_property_parses_slim_and_wide_profiles() {
        let slim_json = r#"{"textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/1a2b3c4d5e","metadata":{"model":"slim"}},"CAPE":{"url":"http://textures.minecraft.net/texture/deadbeef"}}}"#;
        let (sprite, slim) =
            parse_mojang_skin_property(&b64_encode(slim_json.as_bytes())).unwrap();
        assert!(slim);
        assert_eq!(sprite, format!("skins/{}", sha1_hex_utf16le("1a2b3c4d5e")));

        let wide_json = r#"{"textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/abcdef0123"}}}"#;
        let (sprite_w, slim_w) =
            parse_mojang_skin_property(&b64_encode(wide_json.as_bytes())).unwrap();
        assert!(!slim_w);
        assert_eq!(sprite_w, format!("skins/{}", sha1_hex_utf16le("abcdef0123")));
    }

    #[test]
    fn default_skin_for_uuid_covers_both_arm_models() {
        let mut saw_slim = false;
        let mut saw_wide = false;
        for i in 0..36_u128 {
            let (path, slim) = default_skin_for_uuid(i);
            assert!(path.starts_with("entity/player/"));
            if slim {
                saw_slim = true;
            } else {
                saw_wide = true;
            }
        }
        assert!(saw_slim && saw_wide);
    }
}
