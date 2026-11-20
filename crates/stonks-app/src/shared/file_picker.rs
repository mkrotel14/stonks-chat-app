//! Native image selection and source UiImageCropDialog lifecycle.
use crate::runtime::spawn_core;
use futures_channel::oneshot;
use gpui_kit::component::slider::{SliderEvent, SliderState, SliderValue};
use gpui_kit::*;
use std::{
    io::{Cursor, Read},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use stonks_core::uploads::UploadFile;
use stonks_ui::{
    components::{Button, ButtonVariant, Feedback, FeedbackTone, UiCloseButton, UiDialog, UiRange},
    i18n::t,
    Theme,
};
#[derive(Clone)]
pub struct CropService(pub Entity<CropHost>);
impl Global for CropService {}
pub fn install(window: &mut Window, cx: &mut App) {
    let host = cx.new(|cx| CropHost::new(window, cx));
    cx.set_global(CropService(host));
}
type PickResult = Result<Option<UploadFile>, String>;
pub fn pick_image<V: 'static>(
    window: &mut Window,
    cx: &mut Context<V>,
    callback: impl FnOnce(&mut V, PickResult, &mut Window, &mut Context<V>) + 'static,
) -> Task<()> {
    pick(false, window, cx, callback)
}
pub fn pick_cropped_image<V: 'static>(
    window: &mut Window,
    cx: &mut Context<V>,
    callback: impl FnOnce(&mut V, PickResult, &mut Window, &mut Context<V>) + 'static,
) -> Task<()> {
    pick(true, window, cx, callback)
}
fn pick<V: 'static>(
    crop: bool,
    window: &mut Window,
    cx: &mut Context<V>,
    callback: impl FnOnce(&mut V, PickResult, &mut Window, &mut Context<V>) + 'static,
) -> Task<()> {
    let selected = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(t("crop.title").into()),
    });
    cx.spawn_in(window, async move |this, cx| {
        let result = match selected.await {
            Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                Some(path) => spawn_core(async move {
                    tokio::task::spawn_blocking(move || read_image(path))
                        .await
                        .map_err(|_| t("crop.failed"))
                        .and_then(|value| value)
                })
                .await
                .map(Some),
                None => Ok(None),
            },
            Ok(Ok(None)) => Ok(None),
            _ => Err("Could not open the image picker.".into()),
        };
        let result = if crop {
            match result {
                Ok(Some(file)) => {
                    let (sender, receiver) = oneshot::channel();
                    let started = this.update_in(cx, |_, window, cx| {
                        let host = cx.global::<CropService>().0.clone();
                        host.update(cx, |host, cx| host.start(file, sender, window, cx));
                    });
                    if started.is_ok() {
                        receiver.await.unwrap_or(Ok(None))
                    } else {
                        Ok(None)
                    }
                }
                other => other,
            }
        } else {
            result
        };
        this.update_in(cx, |this, window, cx| callback(this, result, window, cx))
            .ok();
    })
}
fn read_image(path: PathBuf) -> Result<UploadFile, String> {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("The selected filename is invalid.")?
        .to_owned();
    let file = std::fs::File::open(path).map_err(|_| "Could not read the selected image.")?;
    let mut bytes = vec![];
    file.take(32_000_001)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the selected image.")?;
    if bytes.len() > 32_000_000 {
        return Err(t("crop.tooLarge"));
    }
    let content_type = match image::guess_format(&bytes) {
        Ok(image::ImageFormat::Png) => "image/png",
        Ok(image::ImageFormat::Jpeg) => "image/jpeg",
        Ok(image::ImageFormat::Gif) => "image/gif",
        Ok(image::ImageFormat::WebP) => "image/webp",
        _ => return Err(t("crop.failed")),
    };
    Ok(UploadFile {
        filename,
        content_type: content_type.into(),
        bytes,
    })
}
fn decode(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| t("crop.failed"))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(160_000_000);
    reader.limits(limits);
    let dimensions = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| t("crop.failed"))?
        .into_dimensions()
        .map_err(|_| t("crop.failed"))?;
    if u64::from(dimensions.0) * u64::from(dimensions.1) > 32_000_000 {
        return Err(t("crop.tooLarge"));
    }
    reader.decode().map_err(|_| t("crop.failed"))
}
fn cropped(image: &image::DynamicImage, zoom: f32, x: f32, y: f32) -> image::DynamicImage {
    let side = (image.width().min(image.height()) as f32 / (zoom / 100.).clamp(1., 4.))
        .round()
        .max(1.) as u32;
    let left = ((image.width() - side) as f32 * x.clamp(0., 100.) / 100.).round() as u32;
    let top = ((image.height() - side) as f32 * y.clamp(0., 100.) / 100.).round() as u32;
    image.crop_imm(left, top, side, side).resize(
        side.min(2048),
        side.min(2048),
        image::imageops::FilterType::Triangle,
    )
}
fn preview(image: &image::DynamicImage) -> Arc<RenderImage> {
    let mut pixels = image
        .resize(512, 512, image::imageops::FilterType::Triangle)
        .to_rgba8();
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]))
}
pub struct CropHost {
    file: Option<UploadFile>,
    image: Option<Arc<image::DynamicImage>>,
    preview: Option<Arc<RenderImage>>,
    ranges: Vec<Entity<SliderState>>,
    reply: Option<oneshot::Sender<PickResult>>,
    focus: FocusHandle,
    opener: Option<FocusHandle>,
    error: Option<String>,
    busy: bool,
    generation: u64,
    work: Option<Task<()>>,
    poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}
impl CropHost {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ranges = [(100., 400., 100.), (0., 100., 50.), (0., 100., 50.)]
            .into_iter()
            .map(|(min, max, value)| {
                cx.new(|_| {
                    SliderState::new()
                        .min(min)
                        .max(max)
                        .step(1.)
                        .default_value(value)
                })
            })
            .collect::<Vec<_>>();
        let subscriptions = ranges
            .iter()
            .map(|range| {
                cx.subscribe(range, |this, _, event, cx| {
                    if matches!(event, SliderEvent::Change(_)) {
                        this.schedule_preview(cx);
                    }
                })
            })
            .collect();
        Self {
            file: None,
            image: None,
            preview: None,
            ranges,
            reply: None,
            focus: cx.focus_handle(),
            opener: None,
            error: None,
            busy: false,
            generation: 0,
            work: None,
            poll: None,
            _subscriptions: subscriptions,
        }
    }
    pub fn is_open(&self) -> bool {
        self.file.is_some()
    }
    pub fn busy(&self) -> bool {
        self.busy
    }
    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.busy {
            self.finish(Ok(None), window, cx);
        }
    }
    pub fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish(Ok(None), window, cx);
    }
    fn finish(&mut self, result: PickResult, window: &mut Window, cx: &mut Context<Self>) {
        self.work = None;
        self.poll = None;
        self.generation = self.generation.wrapping_add(1);
        self.file = None;
        self.image = None;
        self.preview = None;
        self.error = None;
        self.busy = false;
        if let Some(opener) = self.opener.take() {
            opener.focus(window, cx);
        }
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(result);
        }
        cx.notify();
    }
    fn start(
        &mut self,
        file: UploadFile,
        reply: oneshot::Sender<PickResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish(Ok(None), window, cx);
        self.opener = window.focused(cx);
        self.reply = Some(reply);
        self.file = Some(file.clone());
        self.busy = false;
        self.focus.focus(window, cx);
        for (range, value) in self.ranges.iter().zip([100., 50., 50.]) {
            range.update(cx, |range, cx| range.set_value(value, window, cx));
        }
        let generation = self.generation;
        self.work = Some(cx.spawn(async move |this, cx| {
            let decoded = spawn_core(async move {
                tokio::task::spawn_blocking(move || decode(&file.bytes))
                    .await
                    .map_err(|_| t("crop.failed"))
                    .and_then(|result| result)
            })
            .await;
            this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                match decoded {
                    Ok(image) => {
                        this.image = Some(Arc::new(image));
                        this.schedule_preview(cx);
                    }
                    Err(error) => {
                        this.error = Some(error);
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
        self.poll = Some(cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let keep = this
                .update_in(cx, |this, window, cx| {
                    if this.reply.as_ref().is_some_and(|reply| reply.is_canceled()) {
                        this.reset(window, cx);
                        false
                    } else {
                        this.is_open()
                    }
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        }));
        cx.notify();
    }
    fn values(&self, cx: &App) -> (f32, f32, f32) {
        let values = self
            .ranges
            .iter()
            .map(|range| match range.read(cx).value() {
                SliderValue::Single(value) => value,
                _ => 0.,
            })
            .collect::<Vec<_>>();
        (values[0], values[1], values[2])
    }
    fn schedule_preview(&mut self, cx: &mut Context<Self>) {
        let Some(image) = self.image.clone() else {
            return;
        };
        let (z, x, y) = self.values(cx);
        self.work = None;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.work = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(25))
                .await;
            let result = spawn_core(async move {
                tokio::task::spawn_blocking(move || preview(&cropped(&image, z, x, y)))
                    .await
                    .ok()
            })
            .await;
            this.update(cx, |this, cx| {
                if generation == this.generation {
                    this.preview = result;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(image) = self.image.clone() else {
            return;
        };
        let Some(file) = self.file.clone() else {
            return;
        };
        let (z, x, y) = self.values(cx);
        self.busy = true;
        self.work = None;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.work = Some(cx.spawn_in(window, async move |this, cx| {
            let result = spawn_core(async move {
                tokio::task::spawn_blocking(move || {
                    let image = cropped(&image, z, x, y);
                    let mut bytes = Cursor::new(vec![]);
                    image
                        .write_to(&mut bytes, image::ImageFormat::Png)
                        .map_err(|_| t("crop.failed"))?;
                    let bytes = bytes.into_inner();
                    if bytes.len() > 8_000_000 {
                        return Err(t("crop.tooLarge"));
                    }
                    let filename = format!(
                        "{}.png",
                        file.filename
                            .rsplit_once('.')
                            .map(|(base, _)| base)
                            .unwrap_or(&file.filename)
                    );
                    Ok(UploadFile {
                        filename,
                        content_type: "image/png".into(),
                        bytes,
                    })
                })
                .await
                .map_err(|_| t("crop.failed"))
                .and_then(|result| result)
            })
            .await;
            this.update_in(cx, |this, window, cx| {
                if generation != this.generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(file) => this.finish(Ok(Some(file)), window, cx),
                    Err(error) => {
                        this.error = Some(error);
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
        cx.notify();
    }
}
impl Render for CropHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div().absolute().inset_0();
        let Some(file) = self.file.as_ref() else {
            return root;
        };
        let theme = Theme::current(cx);
        let mut body = div().flex().flex_col().gap(px(theme.space4)).child(
            div().flex().justify_end().child(
                UiCloseButton::new("crop-close", theme)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
            ),
        );
        let mut tile = div()
            .id("crop-preview")
            .aria_label(t("crop.preview"))
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h(px(240.))
            .flex_shrink_0()
            .rounded(px(theme.radius_md))
            .overflow_hidden()
            .bg(theme.canvas);
        if let Some(preview) = self.preview.clone() {
            tile = tile.child(
                img(preview)
                    .max_w_full()
                    .max_h_full()
                    .object_fit(ObjectFit::Contain),
            );
        }
        body = body.child(tile);
        for (range, label) in
            self.ranges
                .iter()
                .zip(["crop.zoom", "crop.horizontal", "crop.vertical"])
        {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme.space2))
                    .text_size(px(theme.font_size_sm))
                    .child(t(label))
                    .child(
                        UiRange::new(range, theme)
                            .label(t(label))
                            .disabled(self.busy || self.image.is_none()),
                    ),
            );
        }
        if matches!(file.content_type.as_str(), "image/gif" | "image/webp") {
            body = body.child(
                div()
                    .text_color(theme.text_muted)
                    .text_size(px(theme.font_size_sm))
                    .child(t("crop.animation")),
            );
        }
        if let Some(error) = &self.error {
            body = body.child(Feedback::new(theme, FeedbackTone::Error, error.clone()));
        }
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(theme.space2))
                .child(
                    Button::new("crop-cancel", theme)
                        .variant(ButtonVariant::Secondary)
                        .label(t("common.cancel"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                )
                .child(
                    Button::new("crop-original", theme)
                        .variant(ButtonVariant::Secondary)
                        .label(t("crop.original"))
                        .disabled(self.busy || file.bytes.len() > 8_000_000)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let file = this.file.clone();
                            this.finish(Ok(file), window, cx);
                        })),
                )
                .child(
                    Button::new("crop-confirm", theme)
                        .label(t("crop.confirm"))
                        .disabled(self.busy || self.image.is_none())
                        .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                ),
        );
        root = root.child(
            UiDialog::new("image-crop", theme, t("crop.title"))
                .show_close(false)
                .focus(&self.focus)
                .busy(self.busy)
                .on_close(cx.listener(|this, _, window, cx| this.cancel(window, cx)))
                .child(body),
        );
        root
    }
}
#[cfg(test)]
mod tests {
    use super::cropped;
    #[test]
    fn square_crop_respects_zoom_and_pan() {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(400, 200, |x, _| {
            image::Rgba([if x < 200 { 255 } else { 0 }, 0, 0, 255])
        }));
        let left = cropped(&image, 100., 0., 50.);
        let right = cropped(&image, 100., 100., 50.);
        assert_eq!(left.width(), 200);
        assert_eq!(right.width(), 200);
        assert_eq!(left.to_rgba8().get_pixel(50, 50).0[0], 255);
        assert_eq!(right.to_rgba8().get_pixel(50, 50).0[0], 0);
        assert_eq!(cropped(&image, 200., 50., 50.).width(), 100);
    }
}
