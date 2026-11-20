//! Bundled Inter (OFL — see `assets/fonts/OFL-Inter.txt`).
//!
//! The Tauri app declares "Inter, ui-sans-serif, system-ui" and silently
//! depends on the OS having Inter; here the font is embedded so text metrics
//! are identical on every machine.

use std::borrow::Cow;

use gpui_kit::App;

pub const FAMILY: &str = "Inter";
pub const INLINE_OBJECTS: &str = "Stonks Inline Objects";

static REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
static MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
static SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
static BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");
static ITALIC: &[u8] = include_bytes!("../assets/fonts/Inter-Italic.ttf");

/// Register Inter with the GPUI text system. Call once at startup; then set
/// the family on the window root (`font_family(fonts::FAMILY)`) and it
/// cascades through the element tree.
pub fn register(cx: &mut App) {
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(REGULAR),
            Cow::Borrowed(MEDIUM),
            Cow::Borrowed(SEMIBOLD),
            Cow::Borrowed(BOLD),
            Cow::Borrowed(ITALIC),
            Cow::Borrowed(include_bytes!("../assets/fonts/Stonks-Inline-Objects.ttf")),
        ])
        .expect("embedded Inter fonts are valid");
}
