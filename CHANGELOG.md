# Changelog

## v0.6.0 (2026-11-08)

### New

- Check for updates and a version picker to roll the addon back, and save files with non-UTF-8 text sync again
## v0.5.0 (2026-11-07)

### New

- Addon updates install on their own by default, checked hourly, and the game tells you to /reload
## v0.4.3 (2026-11-07)

### Fixed

- The addon updates while WoW is running, and /reload loads it
- Save-file paths show with Windows backslashes throughout
## v0.4.2 (2026-11-06)

### Fixed

- Sync now says why nothing was sent (no WoW install, no save file, no export yet, switched off, or already uploaded) and Settings shows each save file Companion watches; Windows installs registered under the 64-bit registry key are found too
## v0.4.1 (2026-11-05)

### Fixed

- On Windows the app updates itself by running the release's setup, which closes, replaces and restarts it, and a failed update says why
## v0.4.0 (2026-11-05)

### New

- Windows releases include a setup.exe that installs the app, closes and replaces an older copy, and adds Start menu and uninstall entries

### Fixed

- Opening the app while it runs in the tray brings its window back, even after you closed it, and the tray's Open does too
- The companion only lists WoW Forever folders, not Retail or Classic ones the SideQuest addon doesn't run in
- The addon only shows an update when the installed version is older, and its version no longer reads vv0.5.2
## v0.3.3 (2026-11-05)

### Fixed

- On Windows the room links work again and the minimize, maximize and close buttons sit whole in the window's corner
## v0.3.2 (2026-11-05)

### Fixed

- On Windows the app no longer opens a console window next to it
- The app finds WoW wherever Battle.net installed it, and a WoW folder added from inside _forever_ or AddOns still works
## v0.3.1 (2026-11-05)

### Fixed

- Answered events and successful syncs show a check mark instead of a grey question mark
- Change in Settings → Reminders on the Events tab is a link that opens Settings at Reminders
## v0.3.0 (2026-11-05)

### New

- The app installs its own updates and restarts into them, on by default; Settings > Install app updates automatically turns it off
## v0.2.1 (2026-11-05)

### Fixed

- Messages in the corner wrap onto a second line instead of running off the window, and keep their close button
## v0.2.0 (2026-11-05)

### New

- The Raids tab is now Events, listing every kind of guild event with the site's names (World Boss, PvP, Social, ...), reminders can be picked for each kind, and Sync now shows Syncing while it works

### Fixed

- Sync now always answers: it says when there's nothing new to upload or why it can't sync, and retries an account that's waiting after a failed upload
## v0.1.0 (2026-10-30)

### New

- SideQuest Companion, a native tray app for Windows, macOS and Linux that syncs the SideQuest addon with sidequest-guild.com
