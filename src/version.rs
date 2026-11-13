//! Version parsing and the "is an update offered" rule.

use semver::Version;

/// Parses a version, accepting a leading `v` (as in git tags).
pub fn parse(s: &str) -> Option<Version> {
    let s = s.trim();
    Version::parse(s.strip_prefix('v').unwrap_or(s)).ok()
}

/// True only for a stable `latest` that is strictly newer than `current`.
pub fn offers_update(latest: &Version, current: &Version) -> bool {
    latest.pre.is_empty() && latest > current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_update_cases() {
        let cases = [
            ("3.0.6", "3.0.5", true),
            ("3.0.5", "3.0.5", false),
            ("3.0.5", "3.0.6", false),
            ("3.1.0", "3.1.0-rc.1", true),
            ("3.1.0-rc.2", "3.0.5", false),
        ];
        for (latest, current, expected) in cases {
            let l = parse(latest).unwrap();
            let c = parse(current).unwrap();
            assert_eq!(offers_update(&l, &c), expected, "{latest} vs {current}");
        }
    }

    #[test]
    fn parse_accepts_v() {
        assert!(parse("v3.0.5").is_some());
        assert_eq!(parse("v3.0.5"), parse("3.0.5"));
        assert_eq!(parse("x"), None);
    }
}
