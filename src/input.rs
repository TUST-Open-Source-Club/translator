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
    #[cfg(windows)]
    if !cut_off_utf8(&bytes) {
        if let Some(text) = windows::decode_code_page(&bytes) {
            return text;
        }
    }
    eprintln!("warning: input is not valid UTF-8; invalid bytes were replaced");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// UTF-8 that merely ends in the middle of a character (e.g. cut off by `head -c`):
/// decoding it with the legacy code page would garble the whole text
#[cfg(any(windows, test))]
fn cut_off_utf8(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_err_and(|e| e.error_len().is_none())
}

/// cmd.exe pipes (`echo 你好 | translate`) and legacy "ANSI" text files use the
/// console / system code page (e.g. GBK on Chinese Windows), not UTF-8
#[cfg(windows)]
mod windows {
    const CP_UTF8: u32 = 65001;
    const MB_ERR_INVALID_CHARS: u32 = 0x08;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleOutputCP() -> u32;
        fn GetACP() -> u32;
        fn MultiByteToWideChar(
            code_page: u32,
            flags: u32,
            src: *const u8,
            src_len: i32,
            dst: *mut u16,
            dst_len: i32,
        ) -> i32;
    }

    pub fn decode_code_page(bytes: &[u8]) -> Option<String> {
        // 0 when there is no console (e.g. launched by a browser): use the ANSI code page
        let cp = match unsafe { GetConsoleOutputCP() } {
            0 => unsafe { GetACP() },
            cp => cp,
        };
        if cp == CP_UTF8 {
            return None;
        }
        let len = i32::try_from(bytes.len()).ok()?;
        let convert = |dst: *mut u16, dst_len: i32| unsafe {
            MultiByteToWideChar(cp, MB_ERR_INVALID_CHARS, bytes.as_ptr(), len, dst, dst_len)
        };
        let n = convert(std::ptr::null_mut(), 0);
        if n <= 0 {
            return None;
        }
        let mut wide = vec![0u16; n as usize];
        let n = convert(wide.as_mut_ptr(), n);
        if n <= 0 {
            return None;
        }
        wide.truncate(n as usize);
        Some(String::from_utf16_lossy(&wide))
    }
}

#[cfg(test)]
mod tests {
    use super::{cut_off_utf8, decode};

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

    #[test]
    fn cut_off_utf8_is_still_utf8() {
        let text = "你好世界".as_bytes();
        assert!(cut_off_utf8(&text[..text.len() - 1]));
        // GBK "你好" is invalid UTF-8 right away, not merely cut off
        assert!(!cut_off_utf8(b"\xC4\xE3\xBA\xC3"));
        assert_eq!(decode(text[..text.len() - 1].to_vec()), "你好世\u{FFFD}");
    }
}
