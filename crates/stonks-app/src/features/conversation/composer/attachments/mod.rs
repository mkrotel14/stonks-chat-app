//! Attachment intake, bounded snapshots, and staged previews.
pub(super) mod route;

use route::AttachmentRoute;

use super::Composer;
use crate::runtime::spawn_core;
use gpui_kit::{
    div, img, px, ClipboardEntry, ClipboardItem, Context, ExternalPaths, FontWeight, Image,
    InteractiveElement, IntoElement, ObjectFit, ParentElement, PathPromptOptions, RenderImage,
    StatefulInteractiveElement, Styled, StyledImage, Window,
};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    io::{Cursor, Read},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use stonks_ui::{
    components::{Button, ButtonVariant, Checkbox},
    i18n::t,
    icons::Icon,
    Theme,
};

pub(super) const MAX_ATTACHMENTS: usize = 5;

#[derive(Clone)]
pub struct Attachment {
    pub(super) id: u64,
    pub filename: String,
    pub bytes: Arc<[u8]>,
    pub spoiler: bool,
}

pub(super) struct PickedAttachment {
    pub payload: Attachment,
    pub revealed: bool,
    pub preview: Option<Arc<RenderImage>>,
}

pub(super) enum Selection {
    Paths(Vec<PathBuf>),
    ClipboardImage(Image),
}

impl Selection {
    pub(super) fn read(
        self,
        available: usize,
        limit: u64,
    ) -> Result<Vec<PickedAttachment>, String> {
        if available == 0 {
            return Ok(Vec::new());
        }
        match self {
            Self::Paths(paths) => paths
                .into_iter()
                .take(available)
                .map(|path| read_file(path, limit))
                .collect(),
            Self::ClipboardImage(image) => {
                validate_clipboard_image(&image, limit)?;
                Ok(vec![snapshot(clipboard_filename(&image), image.bytes)])
            }
        }
    }
}

pub(super) fn clipboard_image(item: &ClipboardItem) -> Option<&Image> {
    // If a platform supplies both representations, keep normal text paste semantics.
    if item
        .entries()
        .iter()
        .any(|entry| matches!(entry, ClipboardEntry::String(_)))
    {
        return None;
    }
    item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => Some(image),
        _ => None,
    })
}

fn clipboard_filename(image: &Image) -> String {
    format!("clipboard-image.{}", image.format.extension())
}

pub(super) fn validate_clipboard_image(image: &Image, limit: u64) -> Result<(), String> {
    if image.bytes.is_empty() {
        Err(t("messages.attachmentReadFailed"))
    } else if image.bytes.len() as u64 > limit {
        Err(too_large(&clipboard_filename(image), limit))
    } else {
        Ok(())
    }
}

fn too_large(filename: &str, limit: u64) -> String {
    t("messages.attachmentTooLarge")
        .replace("{limit}", &human_size(limit))
        .replace("{filename}", filename)
}

fn read_file(path: PathBuf, limit: u64) -> Result<PickedAttachment, String> {
    let failed = || t("messages.attachmentReadFailed");
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && name.len() <= 255 && !name.chars().any(char::is_control))
        .ok_or_else(failed)?
        .to_owned();
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // Opening a Unix FIFO must not block before the regular-file check.
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK);
    let file = options.open(&path).map_err(|_| failed())?;
    let metadata = file.metadata().map_err(|_| failed())?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(failed());
    }
    if metadata.len() > limit {
        return Err(too_large(&filename, limit));
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| failed())?;
    if bytes.len() as u64 > limit {
        return Err(too_large(&filename, limit));
    }
    if bytes.is_empty() {
        return Err(failed());
    }
    Ok(snapshot(filename, bytes))
}

fn snapshot(filename: String, bytes: Vec<u8>) -> PickedAttachment {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let format = image::guess_format(&bytes).ok();
    let preview = format.and_then(|format| thumbnail(&bytes, format));
    PickedAttachment {
        payload: Attachment {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            filename,
            bytes: bytes.into(),
            spoiler: false,
        },
        revealed: false,
        preview,
    }
}

fn thumbnail(bytes: &[u8], format: image::ImageFormat) -> Option<Arc<RenderImage>> {
    let (width, height) = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .ok()?;
    if u64::from(width) * u64::from(height) > 16_000_000 {
        return None;
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128_000_000);
    reader.limits(limits);
    let mut rgba = reader.decode().ok()?.thumbnail(240, 144).to_rgba8();
    for pixel in rgba.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

pub(super) fn human_size(size: u64) -> String {
    if size >= 1_000_000 {
        format!("{:.1} MB", size as f64 / 1_000_000.)
    } else if size >= 1_000 {
        format!("{:.1} KB", size as f64 / 1_000.)
    } else {
        format!("{size} B")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AttachmentAccess {
    pub channel: String,
    pub can_send: bool,
    pub can_upload: bool,
    pub size_limit: u64,
}

impl Composer {
    /// Install the account-independent core adapter, not a mock success path.
    pub fn set_attachment_route(&mut self, route: AttachmentRoute, cx: &mut Context<Self>) {
        self.attachment_route = Some(route);
        cx.notify();
    }

    /// Call after `set_channel` and whenever effective permissions change.
    pub fn set_attachment_access(&mut self, access: AttachmentAccess, cx: &mut Context<Self>) {
        if self.state.active_channel() == Some(access.channel.as_str())
            && self.attachment_access.as_ref() != Some(&access)
        {
            self.attachment_access = Some(access);
            cx.notify();
        }
    }

    pub(crate) fn upload_allowed(&self) -> bool {
        self.attachment_access
            .as_ref()
            .is_some_and(|access| access.can_send && access.can_upload && access.size_limit > 0)
    }

    pub(super) fn attachment_limit(&self) -> u64 {
        self.attachment_access
            .as_ref()
            .map(|access| access.size_limit)
            .unwrap_or(0)
    }

    pub(crate) fn can_pick(&self) -> bool {
        self.picker.is_none()
            && self.state.active_channel().is_some()
            && !self.state.sending_here()
            && self.attachment_route.is_some()
            && self.upload_allowed()
            && self.state.files().len() < MAX_ATTACHMENTS
    }

    pub(crate) fn pick_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_pick() {
            return;
        }
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t("messages.chooseAttachments").into()),
        });
        self.stage_attachments(window, cx, async move {
            match selected.await {
                Ok(Ok(paths)) => Ok(paths.map(Selection::Paths)),
                _ => Err(t("messages.attachmentPickerFailed")),
            }
        });
    }

    pub(crate) fn drop_files(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_pick() || paths.paths().is_empty() {
            return;
        }
        let available = MAX_ATTACHMENTS - self.state.files().len();
        let paths = paths.paths().iter().take(available).cloned().collect();
        self.stage_attachments(window, cx, async move { Ok(Some(Selection::Paths(paths))) });
    }

    pub(super) fn paste_image(
        &mut self,
        item: &ClipboardItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(image) = clipboard_image(item) else {
            return false;
        };
        if !self.can_pick() {
            return false;
        }
        // GPUI owns the clipboard payload. Check its size before cloning it for staging.
        let result = validate_clipboard_image(image, self.attachment_limit())
            .map(|()| Some(Selection::ClipboardImage(image.clone())));
        self.stage_attachments(window, cx, async move { result });
        true
    }

    fn stage_attachments(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        selection: impl std::future::Future<Output = Result<Option<Selection>, String>> + 'static,
    ) {
        let navigation = self.state.navigation();
        let available = MAX_ATTACHMENTS - self.state.files().len();
        let limit = self.attachment_limit();
        self.picker = Some(cx.spawn_in(window, async move |this, cx| {
            let result = match selection.await {
                Ok(Some(selection)) => {
                    spawn_core(async move {
                        tokio::task::spawn_blocking(move || selection.read(available, limit))
                            .await
                            .unwrap_or_else(|_| Err(t("messages.attachmentReadFailed")))
                    })
                    .await
                }
                Ok(None) => Ok(Vec::new()),
                Err(error) => Err(error),
            };
            this.update_in(cx, |this, _, cx| {
                this.picker = None;
                if this.upload_allowed()
                    && this.attachment_limit() == limit
                    && !this.state.sending_here()
                {
                    this.state.finish_pick(navigation, result);
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub(super) fn attachment_previews(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("composer-attachments")
            .flex()
            .overflow_x_scroll()
            .gap(px(theme.space2))
            .mx(px(theme.space4))
            .pt(px(theme.space4))
            .children(self.state.files().iter().map(|file| {
                let id = file.payload.id;
                let name = &file.payload.filename;
                let disabled = self.state.sending_here();
                let preview = file.preview.as_ref().map(|preview| {
                    if file.payload.spoiler && !file.revealed {
                        stonks_ui::components::media::concealed_media(
                            ("composer-reveal", id),
                            &theme,
                            cx.listener(move |this, _, _, cx| {
                                this.state.reveal_file(id);
                                cx.notify();
                            }),
                        )
                    } else {
                        div().p(px(theme.space1)).child(
                            img(preview.clone())
                                .w_full()
                                .h(px(112.))
                                .rounded(px(theme.radius_sm))
                                .object_fit(ObjectFit::Cover),
                        )
                    }
                });
                div()
                    .w(px(196.))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .rounded(px(theme.radius_md))
                    .overflow_hidden()
                    .bg(theme.canvas)
                    .border_1()
                    .border_color(theme.outline)
                    .children(preview)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(theme.space1))
                            .px(px(theme.space2))
                            .pt(px(theme.space1))
                            .bg(theme.surface_raised)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(theme.font_size_sm))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.text)
                                            .child(name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(theme.font_size_xs))
                                            .text_color(theme.text_muted)
                                            .child(human_size(file.payload.bytes.len() as u64)),
                                    ),
                            )
                            .child(
                                Button::new(("composer-remove", id), theme)
                                    .variant(ButtonVariant::Close)
                                    .tooltip(
                                        t("messages.removeAttachment").replace("{filename}", name),
                                    )
                                    .leading(Icon::X.render_stroke(theme.text_muted, 18., 1.8))
                                    .disabled(disabled)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.state.remove_file(id);
                                        this.focus(window, cx);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .px(px(theme.space2))
                            .pb(px(theme.space1))
                            .bg(theme.surface_raised)
                            .child(
                                Checkbox::new(("composer-spoiler", id), theme)
                                    .checked(file.payload.spoiler)
                                    .disabled(disabled)
                                    .label(t("messages.markSpoiler"))
                                    .on_change(cx.listener(move |this, checked, _, cx| {
                                        this.state.set_spoiler(id, *checked);
                                        cx.notify();
                                    })),
                            ),
                    )
            }))
    }

    pub(super) fn attachment_notice(&self) -> Option<String> {
        let access = self.attachment_access.as_ref()?;
        if !access.can_send {
            return Some(t("messages.noSendPermission"));
        }
        if self.state.files().is_empty() {
            return None;
        }
        if self.attachment_route.is_none() || access.size_limit == 0 {
            return Some(t("messages.attachmentSendUnavailable"));
        }
        if !access.can_upload {
            return Some(t("messages.noUploadPermission"));
        }
        let limit = self.attachment_limit();
        let oversized = self
            .state
            .files()
            .iter()
            .find(|file| file.payload.bytes.len() as u64 > limit)?;
        Some(
            t("messages.attachmentTooLarge")
                .replace("{limit}", &human_size(limit))
                .replace("{filename}", &oversized.payload.filename),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{clipboard_image, thumbnail, validate_clipboard_image, Selection};
    use gpui_kit::{ClipboardEntry, ClipboardItem, Image};
    use std::io::Cursor;

    fn png() -> Vec<u8> {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        png.into_inner()
    }

    #[test]
    fn clipboard_image_snapshots_bytes_and_preserves_text_fallback() {
        let bytes = png();
        let image = Image {
            format: gpui_kit::ImageFormat::Png,
            bytes: bytes.clone(),
            id: 1,
        };
        let mut text = ClipboardItem::new_string("keep this text".into());
        assert!(clipboard_image(&text).is_none());
        text.entries.push(ClipboardEntry::Image(image.clone()));
        assert!(clipboard_image(&text).is_none());
        assert_eq!(text.text().as_deref(), Some("keep this text"));

        let item = ClipboardItem::new_image(&image);
        let selection = || Selection::ClipboardImage(clipboard_image(&item).unwrap().clone());
        let limit = bytes.len() as u64;
        assert!(selection().read(1, limit - 1).is_err());
        assert!(selection().read(0, limit).unwrap().is_empty());
        let staged = selection().read(1, limit).unwrap().pop().unwrap();
        assert_eq!(staged.payload.filename, "clipboard-image.png");
        assert_eq!(staged.payload.bytes.as_ref(), bytes.as_slice());
        assert!(staged.preview.is_some());
        assert!(validate_clipboard_image(
            &Image {
                bytes: Vec::new(),
                ..image
            },
            limit
        )
        .is_err());
    }

    #[test]
    fn native_paths_share_picker_slot_limits_and_snapshot_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drop.png");
        let bytes = png();
        std::fs::write(&path, &bytes).unwrap();
        let limit = bytes.len() as u64;
        let selection = || Selection::Paths(vec![path.clone(), dir.path().to_owned()]);
        assert!(selection().read(1, limit - 1).is_err());
        assert!(selection().read(2, limit).is_err());
        let staged = selection().read(1, limit).unwrap().pop().unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(staged.payload.filename, "drop.png");
        assert_eq!(staged.payload.bytes.as_ref(), bytes.as_slice());
        assert!(staged.preview.is_some());
    }

    #[test]
    fn thumbnail_supports_local_images_and_falls_back_for_invalid_data() {
        assert!(thumbnail(&png(), image::ImageFormat::Png).is_some());
        assert!(thumbnail(b"not a png", image::ImageFormat::Png).is_none());
    }
}
