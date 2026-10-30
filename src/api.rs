//! Talks to the sidequest-guild.com API: uploads `SQX1:` exports, tests the sync token, and fetches the staged
//! `SQI1:` game import. Mirrors `../addon/uploader/src/sidequest_sync/upload.py`'s error handling.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use thiserror::Error;

/// One `reqwest::Client`, reused across requests: rustls, a 30s timeout, and an identifying user agent.
pub fn client() -> Result<Client> {
    Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(format!(
            "SideQuestCompanion/{} (+https://github.com/SideQuest-Guild/companion)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .context("building HTTP client")
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("The site rejected the export: {0}")]
    BadRequest(String),
    #[error("Not authenticated: your sync token is missing or invalid.")]
    Unauthorized,
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Site returned HTTP {0}: {1}")]
    Server(u16, String),
    #[error("Could not reach {0}: {1}")]
    Network(String, String),
    #[error("Timed out talking to {0}")]
    Timeout(String),
    #[error("Site returned a response that was not valid JSON")]
    InvalidJson,
    #[error("The site doesn't know this request (HTTP 404); it may not be updated yet.")]
    NotFound,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct NewUpdated {
    pub new: u32,
    pub updated: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct NewUpdatedVoided {
    pub new: u32,
    pub updated: u32,
    pub voided: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Updated {
    pub updated: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct New {
    pub new: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SignupCounts {
    pub applied: u32,
    pub skipped_older: u32,
    pub unlinked: u32,
}

/// `event_id` is `null` when the night didn't match a site event.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Attendance {
    pub event_id: Option<String>,
    pub marked: u32,
}

/// The site's response to a successful `/api/epgp/imports` upload. Every section beyond `sessions`/`encounters`/
/// `loot`/`awards`/`unlinked_characters`/`attendance` is optional so an older or newer site's response still
/// deserializes.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ImportResult {
    pub recorder: String,
    pub sessions: NewUpdated,
    pub encounters: NewUpdated,
    pub loot: NewUpdatedVoided,
    pub awards: NewUpdatedVoided,
    pub unlinked_characters: Vec<String>,
    pub attendance: Option<Attendance>,
    pub roster: Option<Updated>,
    pub professions: Option<Updated>,
    pub signups: Option<SignupCounts>,
    pub bank: Option<New>,
    pub announcements: Option<New>,
    pub recruits: Option<NewUpdated>,
    pub member_notes: Option<NewUpdatedVoided>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GameImport {
    pub payload: String,
    pub generated_at: DateTime<Utc>,
}

/// `GET {site}/api/epgp/sync-tokens/me`: who the presented token belongs to, as the website sees them. Nothing
/// about the guild is shown to the UI before this succeeds.
#[derive(Debug, Clone, Deserialize)]
pub struct Me {
    pub display_name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    pub is_officer: bool,
    #[serde(default)]
    pub is_guild_member: bool,
    /// The member's private calendar link, if the site has one on file. Never persisted to `settings.json`.
    #[serde(default)]
    pub calendar_url: Option<String>,
    #[serde(default)]
    pub token_name: String,
}

/// `POST {site}/api/epgp/device-authorizations`: a one-time code pair for browser sign-in.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// One poll of `POST {site}/api/epgp/device-authorizations/token`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceTokenPoll {
    Pending,
    /// Polled faster than the site's interval; wait longer.
    SlowDown,
    Denied,
    /// Expired, already used, or unknown.
    Expired,
    Approved {
        token: String,
    },
}

#[derive(Deserialize)]
struct DeviceTokenBody {
    status: String,
    #[serde(default)]
    token: Option<String>,
}

fn device_authorizations_url(site: &str) -> String {
    format!("{}/api/epgp/device-authorizations", site.trim_end_matches('/'))
}

fn imports_url(site: &str) -> String {
    format!("{}/api/epgp/imports", site.trim_end_matches('/'))
}

fn game_import_url(site: &str) -> String {
    format!("{}/api/epgp/game-import", site.trim_end_matches('/'))
}

fn me_url(site: &str) -> String {
    format!("{}/api/epgp/sync-tokens/me", site.trim_end_matches('/'))
}

fn map_send_error(site: &str, err: reqwest::Error) -> ApiError {
    if err.is_timeout() {
        ApiError::Timeout(site.to_owned())
    } else {
        ApiError::Network(site.to_owned(), err.to_string())
    }
}

/// Pull a human-readable message out of a JSON error body's `detail`, `message` or `error` string field, if any.
fn extract_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let obj = value.as_object()?;
    ["detail", "message", "error"]
        .into_iter()
        .find_map(|key| obj.get(key).and_then(|v| v.as_str()).map(str::to_owned))
}

async fn handle_response<T: for<'de> Deserialize<'de>>(
    site: &str,
    resp: reqwest::Response,
) -> Result<T, ApiError> {
    let status = resp.status();
    if status.is_success() {
        let bytes = resp.bytes().await.map_err(|err| map_send_error(site, err))?;
        return serde_json::from_slice(&bytes).map_err(|_| ApiError::InvalidJson);
    }
    let body = resp.text().await.unwrap_or_default();
    let message = extract_message(&body);
    Err(match status {
        StatusCode::BAD_REQUEST => {
            ApiError::BadRequest(message.unwrap_or_else(|| status.to_string()))
        }
        StatusCode::UNAUTHORIZED => ApiError::Unauthorized,
        StatusCode::FORBIDDEN => ApiError::Forbidden(message.unwrap_or_else(|| {
            "your account is no longer an officer, or the token was revoked.".to_owned()
        })),
        StatusCode::NOT_FOUND => ApiError::NotFound,
        other => ApiError::Server(other.as_u16(), message.unwrap_or_else(|| other.to_string())),
    })
}

/// `POST {site}/api/epgp/imports` with `{"payload": payload}`. Auth is resolved before the payload is decoded, so
/// an empty payload is a side-effect-free way to check the token ([`test_token`]).
pub async fn upload(
    client: &Client,
    site: &str,
    token: &str,
    payload: &str,
) -> Result<ImportResult, ApiError> {
    let resp = client
        .post(imports_url(site))
        .bearer_auth(token)
        .json(&serde_json::json!({ "payload": payload }))
        .send()
        .await
        .map_err(|err| map_send_error(site, err))?;
    handle_response(site, resp).await
}

/// Check whether `token` is valid without uploading anything: an empty payload is auth-checked before it's
/// decoded, so 400 (bad export) means the token is fine, while 401/403 mean the token itself is the problem.
pub async fn test_token(client: &Client, site: &str, token: &str) -> Result<(), ApiError> {
    match upload(client, site, token, "").await {
        Ok(_) | Err(ApiError::BadRequest(_)) => Ok(()),
        Err(err) => Err(err),
    }
}

/// `GET {site}/api/epgp/game-import`. An old site answers 401 to a token-authenticated request here even when the
/// token is valid; callers that already confirmed the token with [`test_token`] should treat `Unauthorized` from
/// this call as "site doesn't support staging yet", not a credential problem.
pub async fn game_import(client: &Client, site: &str, token: &str) -> Result<GameImport, ApiError> {
    let resp = client
        .get(game_import_url(site))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| map_send_error(site, err))?;
    handle_response(site, resp).await
}

/// `GET {site}/api/epgp/sync-tokens/me` with the presented token. 401 means the token is missing, invalid or
/// revoked; 403 means the owner is not (or no longer) a guild member (the body reveals nothing else); 404 means
/// an older site that doesn't have this route yet (callers fall back to [`test_token`]).
pub async fn me(client: &Client, site: &str, token: &str) -> Result<Me, ApiError> {
    let resp = client
        .get(me_url(site))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| map_send_error(site, err))?;
    handle_response(site, resp).await
}

/// `DELETE {site}/api/epgp/sync-tokens/me`: revoke the presenting token. 204 on success, 401 if it was already
/// invalid.
pub async fn revoke_me(client: &Client, site: &str, token: &str) -> Result<(), ApiError> {
    let resp = client
        .delete(me_url(site))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| map_send_error(site, err))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let body = resp.text().await.unwrap_or_default();
    let message = extract_message(&body);
    Err(match status {
        StatusCode::UNAUTHORIZED => ApiError::Unauthorized,
        StatusCode::FORBIDDEN => {
            ApiError::Forbidden(message.unwrap_or_else(|| "forbidden".to_owned()))
        }
        StatusCode::NOT_FOUND => ApiError::NotFound,
        other => ApiError::Server(other.as_u16(), message.unwrap_or_else(|| other.to_string())),
    })
}

/// Start browser sign-in. `device_name` (e.g. the computer's name) is shown on the authorize page and becomes part
/// of the minted token's name. A site without browser sign-in answers 404 ([`ApiError::NotFound`]).
pub async fn start_device_authorization(
    client: &Client,
    site: &str,
    device_name: &str,
) -> Result<DeviceAuthorization, ApiError> {
    let resp = client
        .post(device_authorizations_url(site))
        .json(&serde_json::json!({ "device_name": device_name }))
        .send()
        .await
        .map_err(|err| map_send_error(site, err))?;
    handle_response(site, resp).await
}

/// Ask whether the officer has approved yet. The token is handed over exactly once, on the first poll after approval.
pub async fn poll_device_token(
    client: &Client,
    site: &str,
    device_code: &str,
) -> Result<DeviceTokenPoll, ApiError> {
    let resp = client
        .post(format!("{}/token", device_authorizations_url(site)))
        .json(&serde_json::json!({ "device_code": device_code }))
        .send()
        .await
        .map_err(|err| map_send_error(site, err))?;
    let body: DeviceTokenBody = handle_response(site, resp).await?;
    Ok(match (body.status.as_str(), body.token) {
        ("approved", Some(token)) if !token.is_empty() => DeviceTokenPoll::Approved { token },
        ("pending", _) => DeviceTokenPoll::Pending,
        ("slow_down", _) => DeviceTokenPoll::SlowDown,
        ("denied", _) => DeviceTokenPoll::Denied,
        _ => DeviceTokenPoll::Expired,
    })
}

fn pluralize(n: u32, noun: &str) -> Option<String> {
    (n > 0).then(|| format!("{n} {noun}{}", if n == 1 { "" } else { "s" }))
}

/// Like [`pluralize`], but for a noun that doesn't take an `-s` plural (e.g. "loot").
fn count_invariant(n: u32, noun: &str) -> Option<String> {
    (n > 0).then(|| format!("{n} {noun}"))
}

/// A one-line summary for the history list, e.g. `2 sessions, 14 loot, 31 awards, roster updated`.
pub fn summary(result: &ImportResult) -> String {
    let mut parts = Vec::new();
    parts.extend(pluralize(result.sessions.new + result.sessions.updated, "session"));
    parts.extend(pluralize(result.encounters.new + result.encounters.updated, "encounter"));
    parts.extend(count_invariant(result.loot.new + result.loot.updated, "loot"));
    parts.extend(pluralize(result.awards.new + result.awards.updated, "award"));
    parts.extend(pluralize(result.bank.map(|b| b.new).unwrap_or(0), "bank entry"));
    parts.extend(pluralize(result.announcements.map(|a| a.new).unwrap_or(0), "announcement"));
    parts.extend(pluralize(result.recruits.map(|r| r.new + r.updated).unwrap_or(0), "recruit"));
    parts.extend(pluralize(
        result.member_notes.map(|m| m.new + m.updated).unwrap_or(0),
        "member note",
    ));
    parts.extend(pluralize(result.signups.map(|s| s.applied).unwrap_or(0), "signup"));
    if result.roster.is_some_and(|r| r.updated > 0) {
        parts.push("roster updated".to_owned());
    }
    if result.professions.is_some_and(|p| p.updated > 0) {
        parts.push("professions updated".to_owned());
    }
    if parts.is_empty() { "No changes".to_owned() } else { parts.join(", ") }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    async fn poll_with(body: serde_json::Value) -> DeviceTokenPoll {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/epgp/device-authorizations/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        poll_device_token(&client().unwrap(), &server.uri(), "dc").await.unwrap()
    }

    #[tokio::test]
    async fn device_token_poll_maps_each_status() {
        use serde_json::json;
        assert_eq!(poll_with(json!({"status": "pending"})).await, DeviceTokenPoll::Pending);
        assert_eq!(poll_with(json!({"status": "slow_down"})).await, DeviceTokenPoll::SlowDown);
        assert_eq!(poll_with(json!({"status": "denied"})).await, DeviceTokenPoll::Denied);
        assert_eq!(
            poll_with(json!({"status": "approved", "token": "sqs_x", "name": "n"})).await,
            DeviceTokenPoll::Approved { token: "sqs_x".to_owned() }
        );
        // An approval without a token, or a status this build doesn't know, never counts as signed in.
        assert_eq!(poll_with(json!({"status": "approved"})).await, DeviceTokenPoll::Expired);
        assert_eq!(poll_with(json!({"status": "something_new"})).await, DeviceTokenPoll::Expired);
    }

    #[tokio::test]
    async fn device_authorization_on_an_old_site_is_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/epgp/device-authorizations"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let err =
            start_device_authorization(&client().unwrap(), &server.uri(), "pc").await.unwrap_err();
        assert!(matches!(err, ApiError::NotFound));
    }

    #[tokio::test]
    async fn upload_parses_success_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/epgp/imports"))
            .and(header("authorization", "Bearer sqs_test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "recorder": "Alice",
                "sessions": {"new": 2, "updated": 0},
                "encounters": {"new": 1, "updated": 0},
                "loot": {"new": 14, "updated": 0, "voided": 0},
                "awards": {"new": 31, "updated": 0, "voided": 0},
                "unlinked_characters": [],
                "attendance": null,
                "roster": {"updated": 1}
            })))
            .mount(&server)
            .await;

        let result =
            upload(&client().unwrap(), &server.uri(), "sqs_test", "SQX1:abc").await.unwrap();
        assert_eq!(result.sessions.new, 2);
        assert_eq!(result.loot.new, 14);
        assert_eq!(result.roster.unwrap().updated, 1);
        assert_eq!(summary(&result), "2 sessions, 1 encounter, 14 loot, 31 awards, roster updated");
    }

    /// The exact body the real API returned for the addon's `export_v2.txt` fixture: a night with no matching
    /// event (`attendance.event_id` is null) and a section this build doesn't know (`conflicts`).
    #[test]
    fn parses_a_real_site_response_with_no_matching_event() {
        let body = r#"{"recorder": "Officername-Whitemane", "sessions": {"new": 0, "updated": 1},
            "encounters": {"new": 0, "updated": 1}, "loot": {"new": 0, "updated": 1, "voided": 0},
            "awards": {"new": 0, "updated": 4, "voided": 0},
            "unlinked_characters": ["dps1-whitemane", "healy-whitemane", "tanky-whitemane"],
            "attendance": {"event_id": null, "marked": 0}, "conflicts": [], "roster": {"updated": 3},
            "professions": {"updated": 1}, "signups": {"applied": 0, "skipped_older": 0, "unlinked": 1},
            "bank": {"new": 0}, "announcements": {"new": 0}, "recruits": {"new": 0, "updated": 1},
            "member_notes": {"new": 0, "updated": 1, "voided": 0}}"#;
        let result: ImportResult = serde_json::from_str(body).unwrap();
        assert_eq!(result.attendance, Some(Attendance { event_id: None, marked: 0 }));
        assert_eq!(result.unlinked_characters.len(), 3);
    }

    #[tokio::test]
    async fn upload_maps_400_to_bad_request_with_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/epgp/imports"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({"detail": "not a valid SQX1 export"})),
            )
            .mount(&server)
            .await;

        let err =
            upload(&client().unwrap(), &server.uri(), "sqs_test", "garbage").await.unwrap_err();
        assert!(matches!(&err, ApiError::BadRequest(m) if m == "not a valid SQX1 export"));
    }

    #[tokio::test]
    async fn upload_maps_401_to_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/epgp/imports"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err =
            upload(&client().unwrap(), &server.uri(), "sqs_bad", "SQX1:abc").await.unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized));
    }

    #[tokio::test]
    async fn upload_maps_403_to_forbidden_with_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/epgp/imports"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(serde_json::json!({"detail": "no longer an officer"})),
            )
            .mount(&server)
            .await;

        let err =
            upload(&client().unwrap(), &server.uri(), "sqs_test", "SQX1:abc").await.unwrap_err();
        assert!(matches!(&err, ApiError::Forbidden(m) if m == "no longer an officer"));
    }

    #[tokio::test]
    async fn upload_maps_500_to_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/epgp/imports"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let err =
            upload(&client().unwrap(), &server.uri(), "sqs_test", "SQX1:abc").await.unwrap_err();
        assert!(matches!(err, ApiError::Server(500, _)));
    }

    #[tokio::test]
    async fn test_token_succeeds_on_bad_request_and_fails_on_401() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/epgp/imports"))
            .and(wiremock::matchers::body_json(serde_json::json!({"payload": ""})))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;

        assert!(test_token(&client().unwrap(), &server.uri(), "sqs_ok").await.is_ok());
    }

    #[tokio::test]
    async fn test_token_fails_on_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/epgp/imports"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = test_token(&client().unwrap(), &server.uri(), "sqs_bad").await.unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized));
    }

    #[tokio::test]
    async fn game_import_parses_payload_and_generated_at() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/epgp/game-import"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "payload": "SQI1:xyz",
                "generated_at": "2026-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        let result = game_import(&client().unwrap(), &server.uri(), "sqs_test").await.unwrap();
        assert_eq!(result.payload, "SQI1:xyz");
    }

    #[test]
    fn summary_reports_no_changes_when_everything_is_zero() {
        assert_eq!(summary(&ImportResult::default()), "No changes");
    }

    #[tokio::test]
    async fn me_parses_an_officer() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/epgp/sync-tokens/me"))
            .and(header("authorization", "Bearer sqs_officer"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "display_name": "Alice",
                "avatar_url": "https://example.com/a.png",
                "is_officer": true,
                "is_guild_member": true,
                "calendar_url": "https://example.com/cal.ics",
                "token_name": "Alice's PC"
            })))
            .mount(&server)
            .await;

        let me = me(&client().unwrap(), &server.uri(), "sqs_officer").await.unwrap();
        assert_eq!(me.display_name, "Alice");
        assert!(me.is_officer);
        assert_eq!(me.calendar_url.as_deref(), Some("https://example.com/cal.ics"));
    }

    #[tokio::test]
    async fn me_parses_a_member() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/epgp/sync-tokens/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "display_name": "Bob",
                "avatar_url": null,
                "is_officer": false,
                "is_guild_member": true,
                "calendar_url": null,
                "token_name": "Bob's Laptop"
            })))
            .mount(&server)
            .await;

        let me = me(&client().unwrap(), &server.uri(), "sqs_member").await.unwrap();
        assert_eq!(me.display_name, "Bob");
        assert!(!me.is_officer);
        assert_eq!(me.avatar_url, None);
        assert_eq!(me.calendar_url, None);
    }

    #[tokio::test]
    async fn me_maps_403_to_forbidden_when_not_a_guild_member() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/epgp/sync-tokens/me"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let err = me(&client().unwrap(), &server.uri(), "sqs_ex_member").await.unwrap_err();
        assert!(matches!(err, ApiError::Forbidden(_)));
    }

    #[tokio::test]
    async fn me_maps_404_to_not_found_on_an_older_site() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/epgp/sync-tokens/me"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let err = me(&client().unwrap(), &server.uri(), "sqs_old_site").await.unwrap_err();
        assert!(matches!(err, ApiError::NotFound));
    }

    #[tokio::test]
    async fn me_maps_401_to_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/epgp/sync-tokens/me"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = me(&client().unwrap(), &server.uri(), "sqs_revoked").await.unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized));
    }

    #[tokio::test]
    async fn revoke_me_succeeds_on_204() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/epgp/sync-tokens/me"))
            .and(header("authorization", "Bearer sqs_test"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        revoke_me(&client().unwrap(), &server.uri(), "sqs_test").await.unwrap();
    }

    #[tokio::test]
    async fn revoke_me_fails_on_401_when_already_invalid() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/epgp/sync-tokens/me"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = revoke_me(&client().unwrap(), &server.uri(), "sqs_bad").await.unwrap_err();
        assert!(matches!(err, ApiError::Unauthorized));
    }
}
