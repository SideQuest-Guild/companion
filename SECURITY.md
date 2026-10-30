# Security

## What's in scope

- The sync token and calendar URL the companion stores (see [below](#where-secrets-live)) and everything it does with
  them: uploading addon exports, fetching your private calendar, staging a game import back to the addon.
- Parsing `SideQuest.lua` (the addon's SavedVariables file) and writing the inbox block back into it.
- Verifying addon and self-update downloads before installing them.

## Reporting a problem

Report security problems privately through
[GitHub's private reporting](https://github.com/SideQuest-Guild/companion/security/advisories/new), not in a public
issue. That covers the companion app and anything that could leak a sync token, corrupt a SavedVariables file, or let
a tampered update run as the companion.

## Where secrets live

The sync token and calendar URL go in your OS keyring (Keychain, Windows Credential Manager, or the Secret Service on
Linux) under the service name `com.sidequest-guild.companion`. If no keyring is available, the companion falls back
to a `secrets.json` file in its config directory, created with permissions that only your user can read, and it warns
you in the app when it does. Neither secret is ever written to `settings.json` or to the log files in the log
directory.

## Checking a download

Every release asset is built by GitHub Actions from a signed tag, and GitHub releases can't be changed once
published. To check that an asset you downloaded came from this repo's release workflow, install the
[GitHub CLI](https://cli.github.com/) and run:

```sh
gh attestation verify sidequest-companion-0.1.0-linux-x86_64.tar.gz --repo SideQuest-Guild/companion
```

To check a file against the GitHub release it came from:

```sh
gh release verify-asset v0.1.0 sidequest-companion-0.1.0-linux-x86_64.tar.gz --repo SideQuest-Guild/companion
```

Each release also carries an SPDX SBOM (`.spdx.json`) and `SHA256SUMS.txt`. The companion itself checks both the
addon zip and its own self-update download against the release's `SHA256SUMS.txt` before installing anything, and
refuses to install on a mismatch.

## Versions

Only the latest release gets fixes. The companion checks for a new version twice a day (unless you turn that off in
**Settings**) and tells you when one is out; installing it takes one click.
