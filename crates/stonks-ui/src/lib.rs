//! Stonks design system for GPUI.
//!
//! `theme` is the 1:1 port of `src/design` (tokens, both themes, runtime
//! accent derivation, parity-tested against the TypeScript original).
//! `components` ports `src/ui` controls with the same variants and metrics.

pub mod components;
pub mod external_links;
pub mod fonts;
pub mod i18n;
pub mod icons;
pub mod markdown;
pub mod theme;

pub use theme::{Mode, Theme};
