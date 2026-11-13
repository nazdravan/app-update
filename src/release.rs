//! Parsing of the GitHub `releases/latest` answer.

use semver::Version;
use serde::Deserialize;

use crate::{UpdateError, version};

/// One downloadable file of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// A published, stable release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub notes: String,
    pub page: String,
    pub assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// Parses the JSON of `GET /repos/<repo>/releases/latest`.
///
/// Broken JSON, an unparsable tag, a draft or a pre-release are `BadResponse`.
pub fn parse_latest(json: &str) -> Result<Release, UpdateError> {
    let raw: RawRelease =
        serde_json::from_str(json).map_err(|e| UpdateError::BadResponse(e.to_string()))?;
    if raw.draft || raw.prerelease {
        return Err(UpdateError::BadResponse(
            "release is a draft or a pre-release".into(),
        ));
    }
    let version = version::parse(&raw.tag_name)
        .ok_or_else(|| UpdateError::BadResponse(format!("bad tag: {}", raw.tag_name)))?;
    Ok(Release {
        version,
        tag: raw.tag_name,
        notes: raw.body.unwrap_or_default(),
        page: raw.html_url,
        assets: raw
            .assets
            .into_iter()
            .map(|a| Asset {
                name: a.name,
                url: a.browser_download_url,
                size: a.size,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATEST: &str = include_str!("../tests/fixtures/latest.json");

    #[test]
    fn parse_latest_fixture() {
        let r = parse_latest(LATEST).unwrap();
        assert_eq!(r.version, semver::Version::new(3, 0, 5));
        assert_eq!(r.tag, "v3.0.5");
        assert_eq!(r.assets.len(), 10);
        assert_eq!(
            r.page,
            "https://github.com/nazdravan/furnica/releases/tag/v3.0.5"
        );
        assert!(r.notes.contains("Adăugat"));
        let deb = r.assets.iter().find(|a| a.name.ends_with(".deb")).unwrap();
        assert_eq!(deb.size, 5147012);
        assert!(
            deb.url
                .starts_with("https://github.com/nazdravan/furnica/releases/download/v3.0.5/")
        );
    }

    #[test]
    fn parse_latest_rejects_prerelease_and_garbage() {
        let pre = LATEST.replace("\"prerelease\": false", "\"prerelease\": true");
        assert_ne!(pre, LATEST);
        assert!(matches!(
            parse_latest(&pre),
            Err(UpdateError::BadResponse(_))
        ));
        let draft = LATEST.replace("\"draft\": false", "\"draft\": true");
        assert_ne!(draft, LATEST);
        assert!(matches!(
            parse_latest(&draft),
            Err(UpdateError::BadResponse(_))
        ));
        let bad_tag = LATEST.replace("\"tag_name\": \"v3.0.5\"", "\"tag_name\": \"nightly\"");
        assert_ne!(bad_tag, LATEST);
        assert!(matches!(
            parse_latest(&bad_tag),
            Err(UpdateError::BadResponse(_))
        ));
        assert!(matches!(
            parse_latest("not json"),
            Err(UpdateError::BadResponse(_))
        ));
        assert!(matches!(
            parse_latest("{}"),
            Err(UpdateError::BadResponse(_))
        ));
    }
}
