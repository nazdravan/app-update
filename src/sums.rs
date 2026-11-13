//! Lookup in a `SHA256SUMS.txt` file.

/// Returns the lowercase SHA-256 hex for `name`.
///
/// Accepts `<hex>  <name>` (text mode) and `<hex> *<name>` (binary mode).
pub fn sha256_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, rest) = line.trim_end().split_once(char::is_whitespace)?;
        let file = rest
            .trim_start()
            .strip_prefix('*')
            .unwrap_or(rest.trim_start());
        let valid = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        (valid && file == name).then(|| hash.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMS: &str = include_str!("../tests/fixtures/SHA256SUMS.txt");

    #[test]
    fn sums_lookup() {
        assert_eq!(
            sha256_for(SUMS, "furnica-3.0.5-linux-x86_64.deb").as_deref(),
            Some("398257f06c85b7d3a430e8f46b0193606236ab9fcb99c2024439b1e83ab33224")
        );
        assert_eq!(sha256_for(SUMS, "nope.zip"), None);
        let star = "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789 *a.zip\n";
        assert_eq!(
            sha256_for(star, "a.zip").as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        );
    }

    #[test]
    fn sums_rejects_malformed_and_partial_names() {
        assert_eq!(sha256_for("zzzz  a.zip\n", "a.zip"), None);
        assert_eq!(sha256_for(SUMS, "3.0.5-linux-x86_64.deb"), None);
    }
}
