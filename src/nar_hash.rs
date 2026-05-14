/// Decode a narinfo `NarHash` field into raw SHA-256 bytes.
pub fn sha256_bytes(nar_hash: &str) -> Option<[u8; 32]> {
    let hash = nar_hash.strip_prefix("sha256:").unwrap_or(nar_hash);
    let bytes = if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        hex::decode(hash).ok()?
    } else {
        decode_nix_base32(hash)?
    };
    bytes.try_into().ok()
}

pub fn sha256_vec(nar_hash: &str) -> Vec<u8> {
    sha256_bytes(nar_hash).map(Vec::from).unwrap_or_default()
}

fn decode_nix_base32(input: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 32] = b"0123456789abcdfghijklmnpqrsvwxyz";
    let mut out = vec![0u8; input.len() * 5 / 8];
    for (index, chr) in input.bytes().rev().enumerate() {
        let value = ALPHABET.iter().position(|&c| c == chr)? as u8;
        set_nix_base32_quintet(&mut out, index, value);
    }
    Some(out)
}

fn set_nix_base32_quintet(out: &mut [u8], index: usize, value: u8) {
    let offset = index * 5 / 8;
    let value = value & 0x1f;
    match index % 8 {
        0 => out[offset] |= value,
        1 => {
            out[offset] |= (value & 0x07) << 5;
            set_byte(out, offset + 1, value >> 3);
        },
        2 => out[offset] |= value << 2,
        3 => {
            out[offset] |= (value & 0x01) << 7;
            set_byte(out, offset + 1, value >> 1);
        },
        4 => {
            out[offset] |= (value & 0x0f) << 4;
            set_byte(out, offset + 1, value >> 4);
        },
        5 => out[offset] |= value << 1,
        6 => {
            out[offset] |= (value & 0x03) << 6;
            set_byte(out, offset + 1, value >> 2);
        },
        7 => out[offset] |= value << 3,
        _ => unreachable!(),
    }
}

fn set_byte(out: &mut [u8], offset: usize, value: u8) {
    if let Some(byte) = out.get_mut(offset) {
        *byte |= value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_guix_nix_base32() {
        let bytes =
            sha256_bytes("sha256:0qhasy0w9w9mfv0vacgzymxl4nww8cslyza5x2ci42v7i2b13lyl").unwrap();
        assert_eq!(
            hex::encode(bytes),
            "d4d3119688670b1299e8457d4f35439c5b427bf5ff31b5c17635f1c481d70a62"
        );
    }

    #[test]
    fn accepts_hex() {
        let hash = "d4d3119688670b1299e8457d4f35439c5b427bf5ff31b5c17635f1c481d70a62";
        let bytes = sha256_bytes(&format!("sha256:{hash}")).unwrap();
        assert_eq!(hex::encode(bytes), hash);
    }
}
