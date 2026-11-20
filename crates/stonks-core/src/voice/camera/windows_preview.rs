use super::{CameraError, CameraOptions};
use tokio::sync::{oneshot, watch};

pub struct PreviewFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub async fn run_preview(
    _options: CameraOptions,
    _stop: oneshot::Receiver<()>,
    _frames: watch::Sender<Option<PreviewFrame>>,
) -> Result<(), CameraError> {
    Err(CameraError::CaptureUnavailable)
}
