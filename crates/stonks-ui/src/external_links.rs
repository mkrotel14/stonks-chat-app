//! Host-owned dispatch for every external link activation in native UI.
use gpui_kit::{App, Global, Window};

pub struct ExternalLinkHandler(pub fn(&str, &mut Window, &mut App));
impl Global for ExternalLinkHandler {}

pub fn request(url: &str, window: &mut Window, cx: &mut App) {
    // An unconfigured host must not bypass the link policy.
    if let Some(handler) = cx
        .try_global::<ExternalLinkHandler>()
        .map(|handler| handler.0)
    {
        handler(url, window, cx);
    }
}
