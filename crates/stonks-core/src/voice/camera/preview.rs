use super::{capture::Capture, CameraError, CameraOptions};
use livekit::webrtc::{
    prelude::{I420Buffer, VideoFormatType},
    video_frame::native::VideoFrameBufferExt,
};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, watch};

pub struct PreviewFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Local capture only. The caller owns cancellation; no room or track is involved.
/// The watch slot holds at most one unconsumed frame.
pub async fn run_preview(
    options: CameraOptions,
    mut stop: oneshot::Receiver<()>,
    frames: watch::Sender<Option<PreviewFrame>>,
) -> Result<(), CameraError> {
    let mut capture = tokio::select! {
        biased;
        _ = &mut stop => return Ok(()),
        _ = frames.closed() => return Ok(()),
        capture = Capture::open(&options) => capture?,
    };
    let mut next_frame = Instant::now();
    let result = loop {
        tokio::select! {
            biased;
            _ = &mut stop => break Ok(()),
            _ = frames.closed() => break Ok(()),
            frame = capture.read() => if let Err(error) = frame { break Err(error); },
        }
        let now = Instant::now();
        if now < next_frame {
            continue;
        }
        next_frame = now + Duration::from_millis(66);
        if frames.send(Some(preview_frame(capture.i420()))).is_err() {
            break Ok(());
        }
    };
    capture.close().await;
    result
}

fn preview_frame(mut source: I420Buffer) -> PreviewFrame {
    let (w, h) = (640, 360);
    let mut bgra = vec![0; (w * h * 4) as usize];
    // libyuv ARGB is stored in BGRA byte order on Linux little-endian targets.
    source
        .scale(w, h)
        .to_argb(VideoFormatType::ARGB, &mut bgra, w as u32 * 4, w, h);
    PreviewFrame {
        width: w as u32,
        height: h as u32,
        bgra,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::camera::CameraQuality;

    fn options() -> CameraOptions {
        CameraOptions {
            device: "default".into(),
            quality: CameraQuality::Hd30,
        }
    }

    #[tokio::test]
    async fn cancelled_preview_returns_without_a_frame() {
        let (stop, cancelled) = oneshot::channel();
        let (frames, latest) = watch::channel(None);
        drop(stop);
        assert_eq!(run_preview(options(), cancelled, frames).await, Ok(()));
        assert!(latest.borrow().is_none());
    }

    #[tokio::test]
    async fn preview_without_a_view_returns_without_a_device() {
        let (_stop, cancelled) = oneshot::channel();
        let (frames, latest) = watch::channel(None);
        drop(latest);
        assert_eq!(run_preview(options(), cancelled, frames).await, Ok(()));
    }

    #[test]
    fn synthetic_red_frame_is_scaled_and_stored_as_bgra() {
        let mut source = I420Buffer::new(1280, 720);
        let (y, u, v) = source.data_mut();
        y.fill(82);
        u.fill(90);
        v.fill(240);
        let frame = preview_frame(source);
        assert_eq!(
            (frame.width, frame.height, frame.bgra.len()),
            (640, 360, 640 * 360 * 4)
        );
        assert!(frame.bgra[0] < 5 && frame.bgra[1] < 5 && frame.bgra[2] > 250);
        assert_eq!(frame.bgra[3], 255);
    }
}
