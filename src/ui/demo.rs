//! Fake [`Snapshot`] for `--demo`, so every page can be previewed and screenshotted before the
//! engine (CoreSync/CoreUpdates) is wired up. Keep this: it stays useful after launch too.

use std::path::PathBuf;

use chrono::{Duration, Utc};

use crate::state::*;

/// Which demo snapshot to show, selected by CLI flag (`--demo`, `--demo-member`, `--demo-gate`,
/// `--demo-login`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoMode {
    /// `--demo`: a signed-in officer.
    Officer,
    /// `--demo-member`: a signed-in member (no sync chamber, no upload history).
    Member,
    /// `--demo-gate`: signed out, the gate's default state.
    Gate,
    /// `--demo-login`: the gate mid browser sign-in, waiting on the website.
    Login,
}

impl DemoMode {
    pub fn from_flags(args: &[String]) -> Option<Self> {
        if args.iter().any(|a| a == "--demo-login") {
            Some(Self::Login)
        } else if args.iter().any(|a| a == "--demo-member") {
            Some(Self::Member)
        } else if args.iter().any(|a| a == "--demo-gate") {
            Some(Self::Gate)
        } else if args.iter().any(|a| a == "--demo") {
            Some(Self::Officer)
        } else {
            None
        }
    }

    pub fn snapshot(self) -> Snapshot {
        match self {
            DemoMode::Officer => snapshot(),
            DemoMode::Member => snapshot_member(),
            DemoMode::Gate => snapshot_gate(),
            DemoMode::Login => snapshot_login_waiting(),
        }
    }
}

pub fn snapshot() -> Snapshot {
    let now = Utc::now();

    let pending = PendingCounts {
        sessions: 2,
        encounters: 6,
        loot: 14,
        awards: 31,
        roster: true,
        professions: 3,
        signups: 5,
        bank: 0,
        announcements: 1,
        recruits: 0,
        member_notes: 2,
    };

    let accounts = vec![
        Account {
            name: "DEMOACCOUNT".into(),
            saved_variables: PathBuf::from(
                "/Applications/World of Warcraft/_classic_beta_/WTF/Account/DEMOACCOUNT/SavedVariables/SideQuest.lua",
            ),
            enabled: true,
            modified: Some(now - Duration::minutes(12)),
            pending: Some(pending.clone()),
            export_hash: Some("9f3a2b1c".into()),
            export_uploaded: false,
            inbox_staged: false,
            standings_generated_at: Some(now - Duration::hours(2)),
        },
        Account {
            name: "ALTACCOUNT".into(),
            saved_variables: PathBuf::from(
                "/Applications/World of Warcraft/_classic_beta_/WTF/Account/ALTACCOUNT/SavedVariables/SideQuest.lua",
            ),
            enabled: true,
            modified: Some(now - Duration::days(1)),
            pending: Some(PendingCounts::default()),
            export_hash: Some("1a2b3c4d".into()),
            export_uploaded: true,
            inbox_staged: true,
            standings_generated_at: Some(now - Duration::hours(26)),
        },
    ];

    let installs = vec![WowInstall {
        root: PathBuf::from("/Applications/World of Warcraft"),
        source: InstallSource::Detected,
        flavors: vec![Flavor {
            dir_name: "_classic_beta_".into(),
            path: PathBuf::from("/Applications/World of Warcraft/_classic_beta_"),
            addon: InstalledAddon::Installed {
                version: "0.2.0".into(),
                path: PathBuf::from(
                    "/Applications/World of Warcraft/_classic_beta_/Interface/AddOns/SideQuest",
                ),
            },
            accounts,
        }],
    }];

    let history = vec![
        HistoryEntry {
            at: now - Duration::minutes(12),
            kind: HistoryKind::Upload,
            outcome: Outcome::Success,
            subject: "_classic_beta_ \u{b7} DEMOACCOUNT".into(),
            summary: "2 sessions, 14 loot, 31 awards".into(),
            detail: Some(
                "recorder: Demo1\nsessions: 2 new\nencounters: 6 new\nloot: 14 new\nawards: 31 new\nunlinked_characters: []"
                    .into(),
            ),
        },
        HistoryEntry {
            at: now - Duration::hours(1),
            kind: HistoryKind::ImportStaged,
            outcome: Outcome::Success,
            subject: "_classic_beta_ \u{b7} ALTACCOUNT".into(),
            summary: "standings staged for next login".into(),
            detail: None,
        },
        HistoryEntry {
            at: now - Duration::hours(3),
            kind: HistoryKind::AddonInstalled,
            outcome: Outcome::Success,
            subject: "addon v0.2.0".into(),
            summary: "installed into _classic_beta_".into(),
            detail: None,
        },
        HistoryEntry {
            at: now - Duration::days(1),
            kind: HistoryKind::Upload,
            outcome: Outcome::Failed,
            subject: "_classic_beta_ \u{b7} DEMOACCOUNT".into(),
            summary: "401 Unauthorized".into(),
            detail: Some("the sync token was rejected by sidequest-guild.com".into()),
        },
        HistoryEntry {
            at: now - Duration::days(2),
            kind: HistoryKind::AppUpdated,
            outcome: Outcome::Success,
            subject: "companion v0.1.0".into(),
            summary: "updated from v0.0.9".into(),
            detail: None,
        },
    ];

    let events = EventsState {
        upcoming: vec![
            UpcomingEvent {
                id: "evt-1".into(),
                title: "Hyjal Summit".into(),
                kind: "raid".into(),
                starts_at: now + Duration::hours(6),
                ends_at: Some(now + Duration::hours(9)),
                location: "Hyjal Summit".into(),
                my_status: Some("signed".into()),
                cancelled: false,
                url: "https://sidequest-guild.com/events/evt-1".into(),
            },
            UpcomingEvent {
                id: "evt-2".into(),
                title: "Black Temple".into(),
                kind: "raid".into(),
                starts_at: now + Duration::days(2),
                ends_at: None,
                location: "Black Temple".into(),
                my_status: Some("tentative".into()),
                cancelled: false,
                url: "https://sidequest-guild.com/events/evt-2".into(),
            },
            UpcomingEvent {
                id: "evt-3".into(),
                title: "Zul'Aman".into(),
                kind: "raid".into(),
                starts_at: now + Duration::days(5),
                ends_at: None,
                location: "Zul'Aman".into(),
                my_status: None,
                cancelled: false,
                url: "https://sidequest-guild.com/events/evt-3".into(),
            },
        ],
        fetched_at: Some(now - Duration::minutes(5)),
        error: None,
    };

    let addon_release = ReleaseCheck::Known {
        latest: Release {
            version: "0.3.0".into(),
            tag: "v0.3.0".into(),
            published_at: Some(now - Duration::days(3)),
            notes: "- Adds profession tracking\n- Fixes loot history export for Vanilla realms"
                .into(),
            html_url: "https://github.com/SideQuest-Guild/addon/releases/tag/v0.3.0".into(),
        },
        checked_at: now - Duration::minutes(5),
    };

    Snapshot {
        settings: Settings::default(),
        token: TokenState::Present { masked: "sqs_\u{2026}a1B2".into() },
        calendar_linked: true,
        login: LoginState::Idle,
        account: AccountState::SignedIn(Member {
            display_name: "Kaelthorne".into(),
            avatar_url: None,
            is_officer: true,
        }),
        connection: ConnectionState::Ok,
        sync: SyncStatus {
            phase: SyncPhase::Watching,
            last_success: Some(now - Duration::minutes(12)),
            last_attempt: Some(now - Duration::minutes(12)),
        },
        installs,
        history,
        addon_release,
        app_update: AppUpdate::UpToDate { checked_at: now - Duration::hours(1) },
        events,
        wow_running: false,
        notices: Vec::new(),
        app_version: "0.1.0-demo".into(),
    }
}

/// A variant of [`snapshot`] for previewing the browser sign-in flow's `Waiting` state: no token
/// saved yet, the app is waiting on the user to approve the code on the website.
pub fn snapshot_login_waiting() -> Snapshot {
    let now = Utc::now();
    Snapshot {
        token: TokenState::Missing,
        account: AccountState::SignedOut,
        connection: ConnectionState::Unknown,
        login: LoginState::Waiting {
            user_code: "WXYZ-1234".into(),
            verification_uri_complete: "https://sidequest-guild.com/authorize?user_code=WXYZ-1234"
                .into(),
            expires_at: now + Duration::minutes(8),
        },
        ..snapshot()
    }
}

/// A variant of [`snapshot`] previewing a member (not an officer): no sync chamber, no history of
/// uploads, Overview leads with the next raid and the addon.
pub fn snapshot_member() -> Snapshot {
    Snapshot {
        account: AccountState::SignedIn(Member {
            display_name: "Brightwillow".into(),
            avatar_url: None,
            is_officer: false,
        }),
        token: TokenState::Present { masked: "sqs_\u{2026}c3D4".into() },
        ..snapshot()
    }
}

/// A variant of [`snapshot`] previewing the gate (`AccountState::SignedOut`), the only screen
/// reachable before sign-in.
pub fn snapshot_gate() -> Snapshot {
    Snapshot {
        account: AccountState::SignedOut,
        token: TokenState::Missing,
        login: LoginState::Idle,
        ..snapshot()
    }
}
