//! Full-window, single-image viewer from ImageViewer.tsx. Remote work stays in
//! the account-scoped Images cache; the host owns external-link policy/routing.
use crate::shared::images::{ImageLoadState, Images};
use gpui_kit::base::{FocusTrapElement as _, Scrollbar, ScrollbarMode};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use stonks_ui::{
    components::{Button, ButtonVariant, UiCloseButton},
    i18n::t,
    icons::Icon,
    Theme,
};
mod geometry;
use geometry::{natural_size, Geometry, Zoom};

const SOURCE_SCOPE: &str = "image-viewer";
const DECODE_EDGE: u32 = 4096;
const PREVIEW_EDGE: u32 = 1280;

#[derive(Clone, Debug)]
pub struct ViewerImage {
    pub url: String,
    pub name: String,
    pub dimensions: Option<(u32, u32)>,
    pub preview_url: Option<String>,
}
impl ViewerImage {
    pub fn new(url: impl Into<String>, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            url: url.into(),
            name: if name.is_empty() {
                "Image".into()
            } else {
                name
            },
            dimensions: None,
            preview_url: None,
        }
    }
    /// A separately addressed timeline thumbnail can be reused immediately
    /// while the original URL is fetched. Invalid/duplicate hints are ignored.
    pub fn preview(mut self, url: impl Into<String>) -> Self {
        let url = url.into();
        self.preview_url = (url != self.url && valid_url(&url)).then_some(url);
        self
    }
    fn preview_url(&self) -> Option<&str> {
        self.preview_url
            .as_deref()
            .filter(|url| *url != self.url && valid_url(url))
    }
    fn sources(&self) -> Vec<(String, u32)> {
        let mut sources = vec![(self.url.clone(), DECODE_EDGE)];
        if let Some(url) = self.preview_url() {
            sources.push((url.to_owned(), PREVIEW_EDGE));
        }
        sources
    }
    pub fn dimensions(mut self, width: u32, height: u32) -> Self {
        self.dimensions = (width > 0 && height > 0).then_some((width, height));
        self
    }
}
pub enum ImageViewerEvent {
    Close,
    OpenExternal(String),
}
pub struct ImageViewer {
    images: Entity<Images>,
    target: Option<ViewerImage>,
    zoom: Zoom,
    scroll: ScrollHandle,
    focus: FocusHandle,
    opener: Option<WeakFocusHandle>,
    _images: Subscription,
}
impl EventEmitter<ImageViewerEvent> for ImageViewer {}
impl ImageViewer {
    pub fn new(images: Entity<Images>, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(&images, |this, _, cx| {
            if this.target.is_some() {
                cx.notify();
            }
        });
        cx.on_release(|this, cx| {
            this.images.update(cx, |images, _| {
                images.set_sources(SOURCE_SCOPE, std::iter::empty())
            })
        })
        .detach();
        Self {
            images,
            target: None,
            zoom: Zoom::default(),
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            opener: None,
            _images: observation,
        }
    }
    pub fn is_open(&self) -> bool {
        self.target.is_some()
    }
    pub fn open(&mut self, image: ViewerImage, window: &mut Window, cx: &mut Context<Self>) {
        if self.target.is_none() {
            self.opener = window.focused(cx).map(|focus| focus.downgrade());
        }
        self.zoom = Zoom::default();
        self.scroll = ScrollHandle::new();
        self.images.update(cx, |images, _| {
            images.set_sources(SOURCE_SCOPE, image.sources())
        });
        self.target = Some(image);
        self.focus.focus(window, cx);
        cx.notify();
    }
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.target.is_none() {
            return;
        }
        let had_focus = self.focus.contains_focused(window, cx);
        let opener = self.opener.take().and_then(|focus| focus.upgrade());
        self.clear(cx);
        if had_focus {
            if let Some(opener) = opener {
                opener.focus(window, cx);
            } else {
                window.blur(cx);
            }
        }
        cx.emit(ImageViewerEvent::Close);
    }
    /// Identity/channel teardown clears ownership without returning focus to a
    /// potentially removed attachment or emitting a user-close action.
    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.target.is_none() {
            return;
        }
        let had_focus = self.focus.contains_focused(window, cx);
        self.opener = None;
        self.clear(cx);
        if had_focus {
            window.blur(cx);
        }
    }
    fn clear(&mut self, cx: &mut Context<Self>) {
        self.target = None;
        self.zoom = Zoom::default();
        self.scroll = ScrollHandle::new();
        self.images.update(cx, |images, _| {
            images.set_sources(SOURCE_SCOPE, std::iter::empty())
        });
        cx.notify();
    }
    fn zoom(&mut self, direction: i8, cx: &mut Context<Self>) {
        if self.target.is_some() && self.zoom.change(direction) {
            cx.notify();
        }
    }
    fn open_external(&self, cx: &mut Context<Self>) {
        if let Some(target) = self.target.as_ref().filter(|target| valid_url(&target.url)) {
            cx.emit(ImageViewerEvent::OpenExternal(target.url.clone()));
        }
    }
}
fn valid_url(value: &str) -> bool {
    url::Url::parse(value).ok().is_some_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

#[derive(Debug, PartialEq)]
enum Feedback {
    Loading,
    Failed,
}
fn feedback_for(state: ImageLoadState, has_image: bool) -> Option<Feedback> {
    match state {
        ImageLoadState::Failed => Some(Feedback::Failed),
        _ if !has_image => Some(Feedback::Loading),
        _ => None,
    }
}

impl Render for ImageViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(target) = self.target.as_ref() else {
            return div().into_any_element();
        };
        let target = target.clone();
        let viewport = window.viewport_size();
        // The source media stage remains dark in both application themes.
        // Use the shared dark palette for readable text/control contrast while
        // retaining the active accent/focus tokens.
        let current = Theme::current(cx);
        let mut theme = Theme::dark();
        theme.accent = current.accent;
        theme.focus = current.focus;
        theme.focus_ring = current.focus_ring;
        let (image, state) = self.images.update(cx, |images, cx| {
            let original = images.get(Some(&target.url), DECODE_EDGE, cx);
            let state = images.state(Some(&target.url), DECODE_EDGE);
            let preview = if state != ImageLoadState::Ready {
                target
                    .preview_url()
                    .and_then(|url| images.get(Some(url), PREVIEW_EDGE, cx))
            } else {
                None
            };
            (
                if state == ImageLoadState::Ready {
                    original
                } else {
                    preview.or(original)
                },
                state,
            )
        });
        let decoded = image.as_ref().and_then(|image| {
            let size = image.size(0);
            u32::try_from(size.width.0)
                .ok()
                .zip(u32::try_from(size.height.0).ok())
        });
        let natural = natural_size(target.dimensions, decoded, state == ImageLoadState::Ready);
        let geometry = Geometry::new(
            (f32::from(viewport.width), f32::from(viewport.height)),
            natural,
            self.zoom.value(),
        );
        let has_image = image.is_some();
        let feedback = feedback_for(state, has_image);
        let mut content = div()
            .id("image-viewer-content")
            .relative()
            .flex()
            .flex_col()
            .items_start()
            .flex_shrink_0()
            .w(px(geometry.content.0))
            .h(px(geometry.content.1));
        if let Some(image) = image {
            content = content.child(
                img(image.clone())
                    .id(("image-viewer-image", image.id.0))
                    .aria_label(target.name.clone())
                    .flex_shrink_0()
                    .ml(px(geometry.image_origin.0))
                    .mt(px(geometry.image_origin.1))
                    .w(px(geometry.image.0))
                    .h(px(geometry.image.1))
                    .object_fit(ObjectFit::Contain)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation()),
            );
        }
        let feedback = feedback.map(|feedback| {
            let failed = feedback == Feedback::Failed;
            let mut message = div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(theme.space2))
                .text_color(theme.text_muted)
                .text_size(px(theme.font_size_sm))
                .when(has_image, |message| {
                    message
                        .p(px(theme.space3))
                        .bg(theme.surface)
                        .rounded(px(theme.radius_md))
                        .border_1()
                        .border_color(theme.outline)
                })
                .child(t(if failed {
                    "messages.imageFailed"
                } else {
                    "common.loading"
                }));
            if failed {
                message = message.child(
                    div()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Button::new("image-viewer-retry", theme)
                                .variant(ButtonVariant::Secondary)
                                .label(t("common.retry"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(target) = this.target.as_ref() {
                                        let url = target.url.clone();
                                        this.images.update(cx, |images, cx| {
                                            images.retry(&url, DECODE_EDGE, cx)
                                        });
                                    }
                                })),
                        ),
                );
            }
            div()
                .absolute()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .when(!has_image, |feedback| feedback.inset_0())
                .when(has_image, |feedback| {
                    feedback
                        .left(px(theme.space3))
                        .right(px(theme.space3))
                        .bottom(px(theme.space3))
                })
                .child(message)
        });

        let scroll = div()
            .id("image-viewer-scroll")
            .size_full()
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .child(content);
        let stage = div()
            .absolute()
            .left(px(geometry.insets.side))
            .right(px(geometry.insets.side))
            .top(px(geometry.insets.top))
            .bottom(px(geometry.insets.bottom))
            .child(
                div()
                    .relative()
                    .size_full()
                    .child(scroll)
                    .child(
                        Scrollbar::new(&self.scroll)
                            .mode(ScrollbarMode::Hover)
                            .styles(|styles| styles.thumb(|thumb| thumb.bg(theme.text_muted))),
                    )
                    .children(feedback),
            );
        let glass = Rgba {
            a: 0.76,
            ..theme.canvas
        };
        let glass_shadow = vec![BoxShadow {
            color: theme.shadow_sm.color.into(),
            offset: point(px(0.), px(theme.shadow_sm.dy)),
            blur_radius: px(theme.shadow_sm.blur),
            spread_radius: px(0.),
            inset: false,
        }];
        let metadata = div()
            .id("image-viewer-name")
            .absolute()
            .top(px(geometry.insets.chrome))
            .left(px(geometry.insets.chrome))
            .max_w(px((f32::from(viewport.width) * 0.52).min(680.)))
            .min_w_0()
            .py(px(8.))
            .px(px(12.))
            .border_1()
            .border_color(theme.outline)
            .rounded(px(theme.radius_pill))
            .bg(glass)
            .shadow(glass_shadow.clone())
            .text_color(theme.text)
            .text_size(px(theme.font_size_sm))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_ellipsis().child(target.name.clone()));
        let actions = div()
            .id("image-viewer-actions")
            .role(Role::Navigation)
            .aria_label("Image viewer controls")
            .absolute()
            .top(px(geometry.insets.chrome))
            .right(px(geometry.insets.chrome))
            .flex()
            .flex_shrink_0()
            .gap(px(theme.space1))
            .p(px(theme.space1))
            .border_1()
            .border_color(theme.outline)
            .rounded(px(theme.radius_pill))
            .bg(glass)
            .shadow(glass_shadow)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new("image-viewer-zoom-out", theme)
                    .variant(ButtonVariant::Icon)
                    .tooltip("Zoom out")
                    .leading(Icon::Minus.render_stroke(theme.text, 20., 1.8))
                    .disabled(!self.zoom.can_decrease())
                    .on_click(cx.listener(|this, _, _, cx| this.zoom(-1, cx))),
            )
            .child(
                Button::new("image-viewer-zoom-in", theme)
                    .variant(ButtonVariant::Icon)
                    .tooltip("Zoom in")
                    .leading(Icon::Plus.render_stroke(theme.text, 20., 1.8))
                    .disabled(!self.zoom.can_increase())
                    .on_click(cx.listener(|this, _, _, cx| this.zoom(1, cx))),
            )
            .child(
                Button::new("image-viewer-open", theme)
                    .variant(ButtonVariant::Icon)
                    .tooltip("Open image")
                    .leading(Icon::Download.render_stroke(theme.text, 20., 1.8))
                    .disabled(!valid_url(&target.url))
                    .on_click(cx.listener(|this, _, _, cx| this.open_external(cx))),
            )
            .child(
                UiCloseButton::new("image-viewer-close", theme)
                    .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
            );
        div()
            .id("image-viewer")
            .absolute()
            .inset_0()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(Rgba {
                a: 0.9,
                ..theme.shadow_viewer.color
            })
            .text_color(theme.text)
            .role(Role::Dialog)
            .aria_label(target.name)
            .tab_index(-1)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close(window, cx)),
            )
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape"
                    && !gpui_kit::base::GlobalState::is_in_deferred_context(cx)
                {
                    this.close(window, cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .child(stage)
            .child(metadata)
            .child(actions)
            .focus_trap("image-viewer-focus-trap", &self.focus)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{feedback_for, Feedback, ImageLoadState, ViewerImage};
    #[test]
    fn failed_original_remains_retryable_while_cached_preview_is_visible() {
        assert_eq!(
            feedback_for(ImageLoadState::Failed, true),
            Some(Feedback::Failed)
        );
        assert_eq!(
            feedback_for(ImageLoadState::Failed, false),
            Some(Feedback::Failed)
        );
        assert_eq!(feedback_for(ImageLoadState::Ready, true), None);
        assert_eq!(feedback_for(ImageLoadState::Loading, true), None);
        assert_eq!(
            feedback_for(ImageLoadState::Loading, false),
            Some(Feedback::Loading)
        );
    }
    #[test]
    fn preview_hints_are_valid_http_without_credentials_and_have_owned_sizes() {
        let target = ViewerImage::new("https://cdn.example.test/original.png", "Image")
            .preview("https://cdn.example.test/preview.png");
        assert_eq!(
            target.sources(),
            vec![
                ("https://cdn.example.test/original.png".into(), 4096),
                ("https://cdn.example.test/preview.png".into(), 1280)
            ]
        );
        for url in [
            "javascript:alert(1)",
            "file:///tmp/image.png",
            "https://user:secret@cdn.example.test/image.png",
            "not a url",
        ] {
            assert!(
                ViewerImage::new("https://cdn.example.test/image.png", "Image")
                    .preview(url)
                    .preview_url()
                    .is_none()
            );
        }
        assert_eq!(
            ViewerImage::new("https://cdn.example.test/image.png", "Image")
                .preview("https://cdn.example.test/image.png")
                .sources()
                .len(),
            1
        );
    }
}
