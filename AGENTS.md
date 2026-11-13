# SideQuest Companion

A Rust desktop app (GPUI UI, system tray) that syncs the [SideQuest-Guild/addon](https://github.com/SideQuest-Guild/addon)
WoW addon's save file with [sidequest-guild.com](https://sidequest-guild.com). `README.md` explains the app to
members; keep it current and in plain words.

## Layout

- `src/engine.rs` -- the background engine (`start(repaint) -> EngineHandle`). Owns the watch/sync/update loop and
  talks to the UI only through `Snapshot` (what the UI reads) and `Command` (what the UI sends), both in
  `src/state.rs`.
- `src/api.rs`, `src/sync.rs`, `src/history.rs`, `src/config.rs`, `src/secrets.rs`, `src/wow/` -- the website client,
  upload/stage logic, sync history, settings, keyring access, and SavedVariables/WoW-install discovery.
- `src/release.rs`, `src/addon_install.rs`, `src/self_update.rs`, `src/reminders.rs`, `src/notifications.rs`,
  `src/autostart.rs` -- GitHub release checks, addon installs, self-updates, raid reminders, OS notifications, and
  launch-at-login.
- `src/main.rs`, `src/ui/`, `assets/` -- the GPUI app, window and tray icon.
- `packaging/` -- `packaging/macos/bundle.sh` assembles `SideQuest Companion.app`; both `make bundle-macos` and the
  release workflow call it. `packaging/linux/sidequest-companion.desktop` ships in the Linux tarball.

## Working

- `make install`, then `make ci` (fmt check, clippy with `-D warnings`, tests, `cargo deny check`). Commit and push
  only when `make ci` exits 0.
- Commits are SSH-signed via 1Password. If signing fails, ask the user to unlock it; never turn signing off.
- Commit subjects are Conventional Commits written as plain sentences. They become the changelog members read, so
  say what changed for them.
- Building on Linux needs GPUI's system libraries first (see the `Install Linux build dependencies` step in
  `.github/workflows/ci.yml`); building on macOS needs full Xcode, not just the Command Line Tools, since GPUI
  compiles Metal shaders at build time.

## Releases
Each release adds a section to `CHANGELOG.md`, which GitHub and the docs site show, and members' companions update
to it. Without a tag, every change sits under "Unreleased".
- Cut one when a batch of `feat` or `fix` commits is ready for members.
- Version `v0.MINOR.PATCH`: any `feat` since the last tag bumps MINOR; only fixes bump PATCH. Set the same version in
  `Cargo.toml` first, or the release workflow stops.

```sh
git cliff --tag v0.2.0 -o CHANGELOG.md
git add CHANGELOG.md Cargo.toml Cargo.lock && git commit -m "docs: changelog for v0.2.0"
git tag -s v0.2.0 -m "SideQuest Companion v0.2.0"
git push origin main v0.2.0
gh workflow run build.yml -R SideQuest-Guild/docs   # show it on the docs site now, not at tomorrow's build
```

`v*` tags can't be moved or deleted, and a published release can't change; fix a bad release with a new version.
The first release also needs `"shipped": true` for the companion in the docs repo's `repos.json`.

## The engine-UI contract

The UI (GPUI) never touches the filesystem, network, or keyring directly -- it only calls `EngineHandle::snapshot()`,
`send(Command)` and `shutdown_blocking()`, and redraws when `Repaint` fires. `Snapshot`, `Command`, and every type
they expose live in `src/state.rs`; changing their shape is a cross-cutting change; everyone whose module touches
them needs to agree before it lands.

## Rules that bite

- No `unwrap()`/`expect()` outside tests, except a truly infallible case with a comment explaining why.
- Never log a sync token or calendar URL, not even in a debug build. Secrets go in the OS keyring (service
  `com.sidequest-guild.companion`); the `secrets.json` fallback file is 0600 and still never logged.
- The `SideQuest_Inbox` addon folder the companion writes (`SideQuest_Inbox.toc` and `Inbox.lua`, under each
  flavor's `Interface/AddOns`) is a contract with the addon: exact `.toc` fields (its `## Interface:` line copied
  from the `SideQuest` addon's own `.toc`, and a `## Dependencies: SideQuest` line so WoW nests it under SideQuest
  in the AddOns list and only loads it alongside SideQuest), exact `Inbox.lua` table shape, atomic write, never an
  import older than the `generated_at` already in `Inbox.lua`. It's written whether or not WoW is running -- WoW
  only reads addon files at login and `/reload`, never writes them, so there's nothing to defer. Changing its
  format needs a matching change on the addon side.
- The website deploys independently: tolerate `401`/`403`/`404` from routes it hasn't shipped yet or doesn't permit
  this account on (e.g. the game-import endpoint on an older site, or for a non-officer) instead of treating them
  as fatal errors -- never assume the site has already deployed.
- Release assets, their names, and what's inside them are fixed by the self-updater and addon-installer code that
  consumes them; see `.github/workflows/release.yml` and `packaging/` before changing either.
