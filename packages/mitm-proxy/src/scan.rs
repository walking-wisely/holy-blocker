/// Outcome of a scan operation.
pub enum ScanResult {
    Allow,
    Block { score: u32 },
}

/// Stub — returns `Allow` for every URL until text-policy has a library surface.
pub fn scan_url(_url: &str) -> ScanResult {
    ScanResult::Allow
}

/// Stub — returns `Allow` for every HTML body until text-policy has a library surface.
pub fn scan_body(_html: &str) -> ScanResult {
    ScanResult::Allow
}

/// Stub — returns `Allow` for every image until `packages/image-sandbox` is ready.
pub fn scan_image(_bytes: &[u8]) -> ScanResult {
    ScanResult::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_url_allows_any_input() {
        assert!(matches!(scan_url("https://example.com/path"), ScanResult::Allow));
        assert!(matches!(scan_url(""), ScanResult::Allow));
    }

    #[test]
    fn scan_body_allows_any_input() {
        assert!(matches!(scan_body("<html>hello</html>"), ScanResult::Allow));
        assert!(matches!(scan_body(""), ScanResult::Allow));
    }

    #[test]
    fn scan_image_allows_any_input() {
        assert!(matches!(scan_image(&[0xFF, 0xD8, 0xFF]), ScanResult::Allow));
        assert!(matches!(scan_image(&[]), ScanResult::Allow));
    }
}
