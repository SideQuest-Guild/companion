//! GitHub releases for the addon and the app: fetching the latest release, comparing an installed
//! version against it, and downloading an asset only after its checksum matches `SHA256SUMS.txt`
//! from the same release.

use std::cmp::Ordering;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The addon repository: `GET /repos/{ADDON_REPO}/releases/latest` returns the same signed zip
/// CurseForge serves.
pub const ADDON_REPO: &str = "SideQuest-Guild/addon";
/// This app's own repository.
pub const APP_REPO: &str = "SideQuest-Guild/companion";

const ADDON_MAX_BYTES: u64 = 50 * 1024 * 1024;
const APP_MAX_BYTES: u64 = 200 * 1024 * 1024;
/// `SHA256SUMS.txt` is tiny; refuse to read an implausibly large one.
const CHECKSUMS_MAX_BYTES: u64 = 1024 * 1024;

/// One GitHub release, normalised: `version` has no leading `v`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRelease {
    pub tag: String,
    pub version: String,
    pub name: String,
    pub body: String,
    pub html_url: String,
    pub published_at: Option<DateTime<Utc>>,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

impl GithubRelease {
    pub fn to_state(&self) -> crate::state::Release {
        crate::state::Release {
            version: self.version.clone(),
            tag: self.tag.clone(),
            published_at: self.published_at,
            notes: self.body.clone(),
            html_url: self.html_url.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    published_at: Option<DateTime<Utc>>,
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

fn user_agent() -> String {
    format!(
        "SideQuestCompanion/{} (+https://github.com/SideQuest-Guild/companion)",
        env!("CARGO_PKG_VERSION")
    )
}

/// Fetch the latest release of `repo` (e.g. [`ADDON_REPO`] or [`APP_REPO`]).
pub async fn latest(client: &reqwest::Client, repo: &str) -> Result<GithubRelease> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header(reqwest::header::USER_AGENT, user_agent())
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .with_context(|| format!("requesting the latest release of {repo}"))?;
    check_github_status(&resp, &format!("checking the latest release of {repo}"))?;
    let body: GhRelease =
        resp.json().await.with_context(|| format!("parsing the latest release of {repo}"))?;
    let version = body.tag_name.strip_prefix('v').unwrap_or(&body.tag_name).to_owned();
    Ok(GithubRelease {
        tag: body.tag_name,
        version,
        name: body.name.unwrap_or_default(),
        body: body.body.unwrap_or_default(),
        html_url: body.html_url,
        published_at: body.published_at,
        assets: body
            .assets
            .into_iter()
            .map(|a| Asset { name: a.name, url: a.browser_download_url, size: a.size })
            .collect(),
    })
}

/// How an installed version compares to the latest release's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionStatus {
    UpToDate,
    Outdated,
    Ahead,
    /// The installed `.toc`/binary still has the literal `@project-version@` placeholder: a dev
    /// checkout, not a packaged build.
    Development,
    /// Either version string couldn't be parsed as semver.
    Unknown,
}

/// Compare an installed version (addon `.toc` value or this app's `CARGO_PKG_VERSION`) against a
/// release's version. Tolerant of a leading `v` and of two-part versions like `1.2`.
pub fn compare(installed: &str, latest: &str) -> VersionStatus {
    let installed = installed.trim();
    if installed.eq_ignore_ascii_case("@project-version@") {
        return VersionStatus::Development;
    }
    let (Some(iv), Some(lv)) = (lenient_version(installed), lenient_version(latest)) else {
        return VersionStatus::Unknown;
    };
    match iv.cmp(&lv) {
        Ordering::Less => VersionStatus::Outdated,
        Ordering::Equal => VersionStatus::UpToDate,
        Ordering::Greater => VersionStatus::Ahead,
    }
}

fn lenient_version(raw: &str) -> Option<Version> {
    let raw = raw.trim();
    let raw = raw.strip_prefix('v').unwrap_or(raw);
    if let Ok(v) = Version::parse(raw) {
        return Some(v);
    }
    // Tolerate missing components, e.g. a `.toc` that says `## Version: 1.2`.
    let numeric_dotted = !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit() || c == '.');
    if numeric_dotted {
        let have = raw.split('.').count();
        if have < 3 {
            let mut padded = raw.to_owned();
            for _ in have..3 {
                padded.push_str(".0");
            }
            return Version::parse(&padded).ok();
        }
    }
    None
}

/// Download `asset_name` from `release`, refusing it unless its SHA-256 matches the entry for it
/// in the release's own `SHA256SUMS.txt`. Size-capped (50 MiB for the addon repo, 200 MiB
/// otherwise) so a misbehaving server can't exhaust memory.
pub async fn download_verified(
    client: &reqwest::Client,
    release: &GithubRelease,
    asset_name: &str,
    progress: &(dyn Fn(f32) + Send + Sync),
) -> Result<Vec<u8>> {
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .with_context(|| format!("release {} has no asset named {asset_name}", release.tag))?;
    let checksums_asset =
        release.assets.iter().find(|a| a.name == "SHA256SUMS.txt").with_context(|| {
            format!(
                "release {} is missing SHA256SUMS.txt; refusing to install an unverified download",
                release.tag
            )
        })?;

    let checksums_text = fetch_text(client, &checksums_asset.url, CHECKSUMS_MAX_BYTES).await?;
    let expected = parse_sha256sums(&checksums_text, asset_name)
        .with_context(|| format!("SHA256SUMS.txt has no entry for {asset_name}"))?;

    let max_bytes = max_download_bytes(release);
    let bytes = fetch_binary_capped(client, &asset.url, max_bytes, progress).await?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(&expected) {
        anyhow::bail!("checksum mismatch for {asset_name}: expected {expected}, got {actual}");
    }
    Ok(bytes)
}

fn max_download_bytes(release: &GithubRelease) -> u64 {
    match repo_from_html_url(&release.html_url).as_deref() {
        Some(ADDON_REPO) => ADDON_MAX_BYTES,
        _ => APP_MAX_BYTES,
    }
}

fn repo_from_html_url(html_url: &str) -> Option<String> {
    let rest = html_url
        .strip_prefix("https://github.com/")
        .or_else(|| html_url.strip_prefix("http://github.com/"))?;
    let mut parts = rest.splitn(3, '/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    Some(format!("{owner}/{repo}"))
}

fn parse_sha256sums(text: &str, asset_name: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let hex = parts.next()?;
        let rest = parts.next()?.trim_start();
        let name = rest.strip_prefix('*').unwrap_or(rest);
        if name == asset_name && hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(hex.to_ascii_lowercase());
        }
    }
    None
}

fn check_github_status(resp: &reqwest::Response, what: &str) -> Result<()> {
    let status = resp.status();
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        let reset_at = resp
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<i64>().ok())
            .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0));
        let when = reset_at
            .map(|t| format!(" (resets around {})", t.format("%H:%M UTC")))
            .unwrap_or_default();
        anyhow::bail!("GitHub API rate limit reached while {what}{when}");
    }
    if !status.is_success() {
        anyhow::bail!("GitHub returned {status} while {what}");
    }
    Ok(())
}

async fn fetch_text(client: &reqwest::Client, url: &str, max_bytes: u64) -> Result<String> {
    let bytes = fetch_binary_capped(client, url, max_bytes, &|_| {}).await?;
    String::from_utf8(bytes).with_context(|| format!("{url} is not valid UTF-8"))
}

async fn fetch_binary_capped(
    client: &reqwest::Client,
    url: &str,
    max_bytes: u64,
    progress: &(dyn Fn(f32) + Send + Sync),
) -> Result<Vec<u8>> {
    let mut resp = client
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent())
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?;
    check_github_status(&resp, &format!("downloading {url}"))?;
    let total = resp.content_length();
    if let Some(total) = total
        && total > max_bytes
    {
        anyhow::bail!("{url} is {total} bytes, over the {max_bytes}-byte limit");
    }
    let mut bytes = Vec::with_capacity(total.unwrap_or(0).min(max_bytes) as usize);
    while let Some(chunk) = resp.chunk().await.with_context(|| format!("reading {url}"))? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > max_bytes {
            anyhow::bail!("{url} exceeded the {max_bytes}-byte limit while downloading");
        }
        if let Some(total) = total
            && total > 0
        {
            progress(bytes.len() as f32 / total as f32);
        }
    }
    progress(1.0);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn release_with_assets(base: &str, assets: &[(&str, &str)]) -> GithubRelease {
        GithubRelease {
            tag: "v1.0.0".into(),
            version: "1.0.0".into(),
            name: "Release".into(),
            body: String::new(),
            html_url: "https://github.com/SideQuest-Guild/addon/releases/tag/v1.0.0".into(),
            published_at: None,
            assets: assets
                .iter()
                .map(|(name, p)| Asset { name: (*name).into(), url: format!("{base}{p}"), size: 0 })
                .collect(),
        }
    }

    fn sha256_hex(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    #[tokio::test]
    async fn download_verified_accepts_matching_checksum() {
        let server = MockServer::start().await;
        let payload = b"hello world".to_vec();
        let digest = sha256_hex(&payload);
        Mock::given(method("GET"))
            .and(path("/asset.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.clone()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/SHA256SUMS.txt"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(format!("{digest}  asset.zip\n")),
            )
            .mount(&server)
            .await;
        let release = release_with_assets(
            &server.uri(),
            &[("asset.zip", "/asset.zip"), ("SHA256SUMS.txt", "/SHA256SUMS.txt")],
        );
        let client = reqwest::Client::new();
        let bytes = download_verified(&client, &release, "asset.zip", &|_| {}).await.unwrap();
        assert_eq!(bytes, payload);
    }

    #[tokio::test]
    async fn download_verified_rejects_checksum_mismatch() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/asset.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello world".to_vec()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/SHA256SUMS.txt"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(format!("{}  asset.zip\n", "0".repeat(64))),
            )
            .mount(&server)
            .await;
        let release = release_with_assets(
            &server.uri(),
            &[("asset.zip", "/asset.zip"), ("SHA256SUMS.txt", "/SHA256SUMS.txt")],
        );
        let client = reqwest::Client::new();
        let err = download_verified(&client, &release, "asset.zip", &|_| {}).await.unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "{err}");
    }

    #[tokio::test]
    async fn download_verified_requires_checksums_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/asset.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello world".to_vec()))
            .mount(&server)
            .await;
        let release = release_with_assets(&server.uri(), &[("asset.zip", "/asset.zip")]);
        let client = reqwest::Client::new();
        let err = download_verified(&client, &release, "asset.zip", &|_| {}).await.unwrap_err();
        assert!(err.to_string().to_lowercase().contains("sha256sums"), "{err}");
    }

    #[tokio::test]
    async fn download_verified_requires_checksum_entry_for_asset() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/asset.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello world".to_vec()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/SHA256SUMS.txt"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(format!("{}  other-file.zip\n", "a".repeat(64))),
            )
            .mount(&server)
            .await;
        let release = release_with_assets(
            &server.uri(),
            &[("asset.zip", "/asset.zip"), ("SHA256SUMS.txt", "/SHA256SUMS.txt")],
        );
        let client = reqwest::Client::new();
        let err = download_verified(&client, &release, "asset.zip", &|_| {}).await.unwrap_err();
        assert!(err.to_string().contains("no entry"), "{err}");
    }

    #[test]
    fn parse_sha256sums_accepts_both_formats() {
        let digest_a = "a".repeat(64);
        let digest_b = "b".repeat(64);
        let text = format!("{digest_a}  file-a.zip\n{digest_b} *file-b.zip\n");
        assert_eq!(parse_sha256sums(&text, "file-a.zip"), Some(digest_a));
        assert_eq!(parse_sha256sums(&text, "file-b.zip"), Some(digest_b));
        assert_eq!(parse_sha256sums(&text, "missing.zip"), None);
    }

    #[test]
    fn compare_handles_v_prefix_prerelease_dev_and_garbage() {
        assert_eq!(compare("v1.2.3", "1.2.3"), VersionStatus::UpToDate);
        assert_eq!(compare("1.0.0", "1.2.0"), VersionStatus::Outdated);
        assert_eq!(compare("2.0.0", "v1.2.0"), VersionStatus::Ahead);
        assert_eq!(compare("1.2.0-beta.1", "1.2.0"), VersionStatus::Outdated);
        assert_eq!(compare("@project-version@", "1.2.0"), VersionStatus::Development);
        assert_eq!(compare("not-a-version", "1.2.0"), VersionStatus::Unknown);
        assert_eq!(compare("1.2", "1.2.0"), VersionStatus::UpToDate);
    }
}
