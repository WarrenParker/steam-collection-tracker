//! Small hex helpers, so the app doesn't need another dependency for them.

pub fn encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let data = [0u8, 1, 127, 128, 255];
        assert_eq!(decode(&encode(&data)).unwrap(), data);
        assert_eq!(encode(&[0xab, 0x01]), "ab01");
        assert_eq!(decode("AB01").unwrap(), vec![0xab, 0x01]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(decode("abc").is_none());
        assert!(decode("zz").is_none());
    }
}
