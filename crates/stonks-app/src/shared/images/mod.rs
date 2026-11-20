//! Account-scoped image cache. Completed textures survive channel navigation;
//! pending work follows active surfaces. Four workers, bounded decoded LRU,
//! explicit retries and identity-checked completions keep memory/work bounded.
use crate::session::Session;
use crate::runtime::spawn_core;
use futures_util::future::{AbortHandle, Abortable};
use gpui_kit::{Context, Entity, RenderImage, Subscription, Task};
use std::sync::Arc;
use stonks_core::{http::HttpPolicy, media};

mod cache;
mod decode;
use cache::Cache;
pub use cache::ImageLoadState;

/// Local selection previews share the bounded animation decoder. Only frames
/// cross back to GPUI; callers construct RenderImage on the UI executor.
pub(crate) async fn preview_frames(bytes: Vec<u8>, edge: u32) -> Option<Vec<image::Frame>> {
    spawn_core(async move {
        tokio::task::spawn_blocking(move || {
            decode::decode(&bytes, edge).map(|decoded| decoded.frames)
        })
        .await
        .ok()
        .flatten()
    })
    .await
}

pub fn attachment_url(file: &stonks_core::messages::models::TimelineAttachment) -> Option<&str> {
    if matches!(file.content_type.as_str(), "image/gif" | "image/webp") {
        file.original_url.as_deref().or(file.url.as_deref())
    } else {
        file.url.as_deref().or(file.original_url.as_deref())
    }
}

const MAX_ENTRIES: usize = 256;
const MAX_PENDING: usize = 16;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

struct Cancel(AbortHandle);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct Work {
    _cancel: Cancel,
    _task: Task<()>,
}

pub struct Images {
    policy: Arc<HttpPolicy>,
    cache: Cache<Arc<RenderImage>, Work>,
    workers: Arc<tokio::sync::Semaphore>,
    _session: Subscription,
}

fn identity(session: &Session) -> Option<(String, String)> {
    session
        .snapshot
        .as_ref()
        .zip(session.server.as_ref())
        .map(|(snapshot, server)| {
            (
                server.deployment_url.clone(),
                snapshot.current_user.id.clone(),
            )
        })
}
fn key(url: &str, edge: u32) -> String {
    format!("{edge}:{url}")
}

impl Images {
    pub fn new(policy: Arc<HttpPolicy>, session: &Entity<Session>, cx: &mut Context<Self>) -> Self {
        let mut cache = Cache::new(MAX_ENTRIES, MAX_CACHE_BYTES, MAX_PENDING);
        cache.activate(identity(session.read(cx)));
        let observe = cx.observe(session, |this, session, cx| {
            if this.cache.activate(identity(session.read(cx))) {
                cx.notify();
            }
        });
        Self {
            policy,
            cache,
            workers: Arc::new(tokio::sync::Semaphore::new(4)),
            _session: observe,
        }
    }

    /// Active scopes protect their cache entries preferentially and own pending
    /// requests. Completed entries remain reusable after a scope navigates away.
    pub fn set_sources(
        &mut self,
        scope: &'static str,
        sources: impl IntoIterator<Item = (String, u32)>,
    ) {
        self.cache.set_sources(
            scope,
            sources.into_iter().map(|(url, edge)| key(&url, edge)),
        );
    }

    pub fn state(&self, url: Option<&str>, edge: u32) -> ImageLoadState {
        url.map_or(ImageLoadState::Missing, |url| {
            self.cache.state(&key(url, edge))
        })
    }

    /// A warm hit returns the same decoded texture synchronously, without a
    /// request, decode job, loading frame or replacement image identity.
    pub fn get(
        &mut self,
        url: Option<&str>,
        edge: u32,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        let url = url?;
        let key = key(url, edge);
        if let Some(image) = self.cache.get(&key) {
            return Some(image);
        }
        let fallback = self.cache.get_matching(|key| {
            key.split_once(':')
                .is_some_and(|(_, cached_url)| cached_url == url)
        });
        if self.cache.state(&key) != ImageLoadState::Missing {
            return fallback;
        }
        let Some(ticket) = self.cache.reserve(key) else {
            return fallback;
        };
        let policy = self.policy.clone();
        let workers = self.workers.clone();
        let request_url = url.to_owned();
        let (abort, registration) = AbortHandle::new_pair();
        let completion = ticket.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = spawn_core(Abortable::new(
                async move {
                    let permit = workers.acquire_owned().await.ok()?;
                    let bytes = media::fetch_image(&policy, &request_url).await.ok()?;
                    tokio::task::spawn_blocking(move || {
                        let _permit = permit;
                        decode::decode(&bytes, edge)
                    })
                    .await
                    .ok()
                    .flatten()
                },
                registration,
            ))
            .await
            .ok()
            .flatten();
            this.update(cx, |this, cx| {
                let result = result
                    .map(|decoded| (Arc::new(RenderImage::new(decoded.frames)), decoded.bytes));
                if this.cache.complete(&completion, result) {
                    cx.notify();
                }
            })
            .ok();
        });
        self.cache.attach(
            &ticket,
            Work {
                _cancel: Cancel(abort),
                _task: task,
            },
        );
        fallback
    }

    pub fn failed(&self, url: Option<&str>, edge: u32) -> bool {
        self.state(url, edge) == ImageLoadState::Failed
    }
    pub fn retry(&mut self, url: &str, edge: u32, cx: &mut Context<Self>) {
        self.cache.retry(&key(url, edge));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::decode::decode;
    use std::io::Cursor;
    #[test]
    fn decodes_bgra_and_rejects_invalid_data() {
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let decoded = decode(bytes.get_ref(), 1280).unwrap();
        assert_eq!(
            decoded.frames[0].buffer().get_pixel(0, 0).0,
            [0, 0, 255, 255]
        );
        assert_eq!(decoded.bytes, 8);
        assert!(decode(b"not an image", 1280).is_none());
    }
    #[test]
    fn bounds_decoded_dimensions_and_thumbnail_size() {
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::new(5000, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(decode(bytes.get_ref(), 80).is_none());
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::new(200, 100)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let decoded = decode(bytes.get_ref(), 80).unwrap();
        assert_eq!(decoded.frames[0].buffer().dimensions(), (80, 40));
        assert_eq!(decoded.bytes, 80 * 40 * 4);
    }
}
