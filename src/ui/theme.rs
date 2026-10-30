//! Brand theme: registers the OFL fonts (Libertinus Serif Display for headings, EB Garamond for
//! body) and applies `assets/theme/sidequest.json` (a GPUI Kit `ThemeConfig`) built from the
//! guild's night/royal/stone/gold palette shared with the addon and the website.

use std::borrow::Cow;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::App;
use gpui_kit::component::theme::{Theme, ThemeConfig};

/// Display face used for the wordmark, ribbon titles and sub-headings (tracked caps).
pub const DISPLAY_FONT: &str = "Libertinus Serif Display";
/// Body face used for everything else.
pub const BODY_FONT: &str = "EB Garamond";

const DISPLAY_TTF: &[u8] = include_bytes!("../../assets/fonts/LibertinusSerifDisplay.ttf");
const BODY_TTF: &[u8] = include_bytes!("../../assets/fonts/EBGaramond.ttf");
const BODY_ITALIC_TTF: &[u8] = include_bytes!("../../assets/fonts/EBGaramond-Italic.ttf");

const THEME_JSON: &str = include_str!("../../assets/theme/sidequest.json");

/// Brand palette. Mirrors `../addon/DESIGN.md` / `../website/DESIGN.md` and `assets/theme/sidequest.json`.
pub mod palette {
    use gpui_kit::{Hsla, Rgba, rgb, rgba};

    pub fn night() -> Hsla {
        rgb(0x0a0f1a).into()
    }
    pub fn night2() -> Hsla {
        rgb(0x101a2c).into()
    }
    pub fn night3() -> Hsla {
        rgb(0x18253d).into()
    }
    pub fn royal() -> Hsla {
        rgb(0x22386a).into()
    }
    pub fn royal2() -> Hsla {
        rgb(0x2e4a85).into()
    }
    pub fn royal_deep() -> Hsla {
        rgb(0x18295a).into()
    }
    pub fn stone() -> Hsla {
        Rgba { r: 150. / 255., g: 172. / 255., b: 210. / 255., a: 0.26 }.into()
    }
    pub fn stone2() -> Hsla {
        Rgba { r: 150. / 255., g: 172. / 255., b: 210. / 255., a: 0.13 }.into()
    }
    pub fn well() -> Hsla {
        Rgba { r: 6. / 255., g: 9. / 255., b: 16. / 255., a: 0.62 }.into()
    }
    pub fn gold() -> Hsla {
        rgb(0xd8ab5c).into()
    }
    pub fn gold_deep() -> Hsla {
        rgb(0xa88449).into()
    }
    pub fn gold_inner() -> Hsla {
        Rgba { r: 168. / 255., g: 132. / 255., b: 73. / 255., a: 0.34 }.into()
    }
    pub fn ink() -> Hsla {
        rgb(0xf2efe9).into()
    }
    pub fn mist() -> Hsla {
        rgb(0xc4cddd).into()
    }
    pub fn mist2() -> Hsla {
        rgb(0x96a2b8).into()
    }
    pub fn warm_error() -> Hsla {
        rgb(0xe0a479).into()
    }
    pub fn flavor() -> Hsla {
        rgba(0xffd36aff).into()
    }
}

/// Raw RGB bytes for brand colors, used where a plain pixel buffer is composited outside GPUI's
/// own color types (e.g. the tray icon's status dot). Mirrors [`palette`].
#[cfg(not(target_os = "macos"))]
pub mod palette_bytes {
    pub const GOLD: [u8; 3] = [0xd8, 0xab, 0x5c];
    pub const MIST2: [u8; 3] = [0x96, 0xa2, 0xb8];
    pub const WARM_ERROR: [u8; 3] = [0xe0, 0xa4, 0x79];
}

/// Registers the OFL fonts and applies the brand theme. Call once, right after `gpui_kit::init(cx)`
/// and before the first window opens or measures text.
pub fn install(cx: &mut App) {
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(DISPLAY_TTF),
            Cow::Borrowed(BODY_TTF),
            Cow::Borrowed(BODY_ITALIC_TTF),
        ])
        .expect("failed to register SideQuest OFL fonts");

    let config: ThemeConfig = serde_json::from_str(THEME_JSON)
        .expect("assets/theme/sidequest.json must parse as a GPUI Kit ThemeConfig");
    let config = Rc::new(config);
    Theme::update(cx, |theme| theme.apply_config(&config));

    refresh_system_reduce_motion();
}

static SYSTEM_REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

/// Re-reads macOS's system "Reduce motion" setting (`NSWorkspace.sharedWorkspace
/// .accessibilityDisplayShouldReduceMotion`, read cheaply here via `defaults` rather than linking
/// AppKit) and caches it. Call once at startup and again whenever the window regains activation,
/// the cheapest point at which the user could plausibly have just changed it in System Settings.
/// A no-op on other platforms, which only honour `Settings::reduce_motion`.
pub fn refresh_system_reduce_motion() {
    #[cfg(target_os = "macos")]
    {
        let reduce = std::process::Command::new("defaults")
            .args(["read", "com.apple.universalaccess", "reduceMotion"])
            .output()
            .map(|out| out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "1")
            .unwrap_or(false);
        SYSTEM_REDUCE_MOTION.store(reduce, Ordering::Relaxed);
    }
}

/// The last value `refresh_system_reduce_motion` read. Always `false` on non-macOS platforms.
pub fn system_reduce_motion() -> bool {
    SYSTEM_REDUCE_MOTION.load(Ordering::Relaxed)
}
