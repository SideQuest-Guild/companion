---
name: SideQuest Companion
description: The guild's tray companion, drawn as rooms of the same citadel as sidequest-guild.com.
inherits: ../website/website/DESIGN.md
colors:
  night: "#0a0f1a"
  night-2: "#101a2c"
  night-3: "#18253d"
  royal: "#22386a"
  royal-2: "#2e4a85"
  royal-deep: "#18295a"
  stone: "rgba(150, 172, 210, 0.26)"
  stone-2: "rgba(150, 172, 210, 0.13)"
  well: "rgba(6, 9, 16, 0.62)"
  gold: "#d8ab5c"
  gold-deep: "#a88449"
  gold-inner: "rgba(168, 132, 73, 0.34)"
  ink: "#f2efe9"
  ribbon-ink: "#f6f3ee"
  mist: "#c4cddd"
  mist-2: "#96a2b8"
  flavor: "#ffd36a"
  error: "#e0a479"
  q-epic: "#a335ee"
typography:
  display: "Libertinus Serif Display 400, uppercase, tracked 0.24-0.54em"
  body: "EB Garamond 400-600 (+ italic face)"
---

# Design System: SideQuest Companion

## Overview

**North star: "A Lantern in the Citadel."** The website is a painted night at the guild gate; the companion is the
same citadel seen from inside, one room per page. Every page opens on its room's painting, which runs up behind the
native title bar and fades into night; the working parts sit in framed chambers drawn the way the game draws its
windows. The website's DESIGN.md is the parent authority for colors, type, ornament and voice; this file adapts it
to a native window and records what is specific to the app.

Rejected (user, 2026-10-30): the generic AI-app look. Concretely: rounded cards, rounded pills and badges, iOS-style
toggle switches, an icon-plus-label sidebar, lucide/glyph icons, full-width colored header bars, toast cards with
shadows, and a custom window outline that collides with the native frame.

## Rooms (pages)

| Page | Room painting (from `../website/website/assets/rooms/`) | Lead content |
|---|---|---|
| Gate (signed out) | `plates/hero-art.png` (the guild gate) | Crest, wordmark, the one gold plaque: Sign in |
| Overview | The Courier's Loft (`assets/rooms/courier-loft`, made for the app; prompt in SideQuest-Guild/.github `art/companion.txt`) | Officer: the sync chamber. Member: next raid + addon |
| History | `ledger-chamber` | The ledger: quest-log rows of uploads/imports/installs |
| Addon | `armory` | Item-tooltip card per game flavor, release notes |
| Events | `raid-hall` | Quest log of upcoming guild events (every kind), reminders |
| Settings | `round-table` | Chambers: Account, WoW folders, Sync, Updates, Reminders, App |

- **Room backdrop (user, 2026-10-30: "less banner space, make the things go up so it's more backgrounded"):** the
  painting is a backdrop, not a banner. It starts at y=0 behind the title bar and covers about half the window,
  dissolving into night over its lower 60%; a top scrim (night at 70% to transparent over 56 px) keeps the title bar
  legible. The room's name sits compact (display caps ~18 px, 0.3em, flanked by short hairline rules ending in outline
  diamonds) right under the nav row, and the first chamber starts about 110-120 px from the top, drawn over the
  painting. Chamber fills stay at 92-95% so text contrast holds; the painting shows in the gaps and margins. No lede,
  no eyebrow.
- **Walking between rooms (the one authored moment):** on page change the old painting scales 1.00 to 1.04 and fades
  out over 420 ms while the new one starts at 1.06 and settles to 1.00 over 900 ms (exponential ease-out); the room
  title lifts 6 px and settles. While the content scrolls, the painting drifts down at 0.3x and fades toward night.
  With Settings › App › Reduce motion on (default follows macOS "Reduce motion" when readable), pages switch
  instantly. Nothing else animates except spinners for real waits and the toast fade.

## Window chrome (native)

- **macOS:** transparent title bar, native traffic lights at their standard position over the painting. Nothing is
  drawn under or around them: no outline, no chamfer, no frame line hugging the window edge. The window keeps the
  system's corner radius and shadow (the one place a radius exists, because it's the OS's).
- **Windows / Linux:** the same painted head with caption buttons drawn by us at the right: 46x32 hit areas, 1px
  gold-deep line glyphs (minimize bar, maximize square, close cross) in ink, hover fills stone-2, close hover fills
  error at 80% with night glyph. Native resize edges stay.
- **Navigation:** one row over the painting, like the website's nav: crest (24 px) and the wordmark
  `SIDEQUEST COMPANION` in display caps (0.54em) at left, after the traffic lights on macOS; room links at right in
  EB Garamond title case, 0.07em, mist at rest, ink on hover, the current room ink with a 1px gold-deep underline.
  44 px tall hit areas. No sidebar, no icons in the nav.

## Chambers

The only container. Exactly as `citadel.css` `.chamber` draws it (read it): night-3 to night gradient fill at
92-95%, 1px stone outer line, 1px `gold-inner` line 5 px inside, and bracketed gold corners: an L with a cut corner
plus a small solid diamond (the 26x26 SVG paths in `citadel.css` lines 171-177, scaled to 20 px). The heading is a
**ribbon** centred on the top edge: royal-2 to royal to royal-deep vertical gradient, gold-deep 1px top and bottom
lines, notched ends (the polygon `0 0, 100% 0, calc(100% - 18px) 50%, 100% 100%, 0 100%, 18px 50%`), display caps
at 0.28em, ribbon ink with a 1px dark text shadow. Nothing sits above a ribbon. Chambers never nest. Two chambers
side by side share one row height.

Sub-headings inside a chamber: display caps (0.24em) followed by a gold-deep line fading out to the right.

## Shapes, depth, icons

- **No corner radius anywhere we draw.** Buttons, wells, tabs, chips, toasts, tooltips: hard rectangles.
- **No box shadows.** Depth comes from the painting's mask, scrims, the chamber's double line, and text shadow on
  text over paint.
- **Icons are drawn symbols, never a stock icon font:** the quest marks (gold "!" for open/needs action, dim "?"
  for done), role marks, the outline diamond, the four-point star, caret chevrons in a 1px line. Paint them with
  paths in one 1.2 px gold or mist stroke. Delete every lucide/gpui-component icon from the UI.

## Components

- **Plaque (primary, one per screen at most):** the gold `cta-plate.png` art behind a near-black (`#17130c`) caps
  label, EB Garamond 600, 0.2em. Used only for Sign in on the gate and Install update when an update is the screen's
  point.
- **Button (`.btn`):** royal-2 to royal fill, 1px gold-deep edge, ink caps at 0.14em, 40 px tall (44 px hit area).
  Hover: brighter fill, edge and label turn gold. Pressed: royal-deep. Disabled: stone-2 edge, mist-2 label, and the
  reason in a mist line beside or under it.
- **Quiet button (`.btn-quiet`):** well fill, 1px stone edge, mist label; hover warms the edge toward gold.
- **Danger:** a quiet button whose edge and label warm to error on hover (Sign out, Remove folder).
- **Wells (inputs):** `well` fill, 1px stone edge, no radius; hover edge warms, focus edge is gold. Placeholder mist-2.
  Caret and selection in gold at 35%.
- **Tabs / filters:** 36 px dark wells with stone edges; the current one takes the royal fill with a gold-deep edge.
- **Checks instead of switches:** a 16 px square well with a stone edge; checked = gold-deep edge with a 1px gold
  check stroke. Label to its right in body type. No pill toggles.
- **State tags (instead of pills):** tracked label caps (0.18em, 12-13 px) in a color with a preceding 5 px diamond:
  gold diamond = needs you / pending; mist = done / quiet; error = failed. No filled backgrounds.
- **Quest log rows (History, Events, accounts):** quest mark at left, title in display caps (0.14em, long titles
  wrap), a mist meta line, time over day at the right in tabular numerals. Hover tints royal at 25% and turns the
  title gold. Rows divide with stone-2 hairlines. Expanded rows show detail as quoted text in flavor gold italic.
- **Item tooltip (Addon):** the addon name in epic quality color, a "Version 0.3.0" line, installed vs latest as
  label/value rows, release notes as flavor text, then the action.
- **Toasts:** bottom-right, a hard-edged night-2 panel with a 1px gold-inner edge and a 5 px diamond in the level's
  color; fade in/out 200 ms. Never cover the nav or a ribbon.
- **Freshness line:** mist italic, e.g. "Synced 12 minutes ago". Shown in every chamber whose data ages.

## Gate (signed out)

Full window: the gate painting, scrimmed; centred crest (`plates/nav-crest.png`), the wordmark in display caps,
one line of body copy ("Sign in with your SideQuest account to start."), the gold plaque "Sign in with
sidequest-guild.com". While waiting: the code in display caps at 32 px, tracked 0.3em, with "Check that the website
shows the same code, then click Authorize." and a countdown; quiet buttons "Open the page again" and "Cancel". Below,
a small quiet link "Use a sync token instead". A signed-in account that isn't in the guild sees: "This app is for
SideQuest members." and a Discord link. Nothing else is reachable from the gate (the tray menu offers only Open and
Quit while signed out).

## Tray quick panel

A 320x380 borderless popover drawn as a single chamber with a 72 px strip of the Courier's Loft painting at its top, a
ribbon with the app's state ("WATCHING", "PAUSED", "SIGN IN"), the freshness line, Sync now (button) and Pause
(quiet), the next raid as one quest-log row, the addon line, and "Open SideQuest Companion" (quiet). Officer-only
rows hide for members.

## Type scale (px at 1.0 zoom)

Wordmark 15 (0.54em). Room title 26 (0.3em). Ribbon 14 (0.28em). Sub-heading 13 (0.24em). Body 17 / line 1.45.
Meta and labels 14. Tags 12.5 (0.18em). Numerals tabular everywhere they line up.

## Do / Don't

- Do keep gold to lines, diamonds, stars, text accents and the one plaque.
- Do show the reason next to every disabled control.
- Don't add a rounded rectangle, a pill, a switch, a shadow, an eyebrow, or a stock icon.
- Don't draw anything around the macOS traffic lights or along the window's outer edge.
- Don't fabricate guild facts; demo data is labelled "Demo data" in the nav row.
