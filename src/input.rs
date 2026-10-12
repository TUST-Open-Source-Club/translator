//! Decoding of raw stdin bytes into text

/// Decode input text. Byte-order marks are honoured (Windows editors and
/// PowerShell redirection write UTF-8 BOM / UTF-16 files); otherwise UTF-8 is
/// expected, with invalid bytes replaced instead of failing the whole run.
pub fn decode(bytes: Vec<u8>) -> String {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return utf16(rest, u16::from_be_bytes);
    }
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => not_utf8(e.into_bytes()),
    }
}

fn utf16(bytes: &[u8], from_bytes: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| from_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

fn not_utf8(bytes: Vec<u8>) -> String {
    eprintln!("warning: input is not valid UTF-8; invalid bytes were replaced");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn plain_utf8() {
        assert_eq!(decode("你好 hello".into()), "你好 hello");
    }

    #[test]
    fn utf8_bom_is_stripped() {
        assert_eq!(decode(b"\xEF\xBB\xBFhi".to_vec()), "hi");
    }

    #[test]
    fn utf16_with_bom() {
        let le: Vec<u8> = [0xFF, 0xFE, 0x60, 0x4F, 0x7D, 0x59].into();
        let be: Vec<u8> = [0xFE, 0xFF, 0x4F, 0x60, 0x59, 0x7D].into();
        assert_eq!(decode(le), "你好");
        assert_eq!(decode(be), "你好");
    }

    #[test]
    fn invalid_utf8_is_replaced() {
        assert_eq!(decode(b"ab\xFFcd".to_vec()), "ab\u{FFFD}cd");
    }
}
