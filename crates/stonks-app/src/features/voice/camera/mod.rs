mod setup;

use super::Voice;
use stonks_core::voice::{
    camera::{CameraOptions, CameraState},
    VoiceSession,
};
use tokio::sync::{mpsc, watch};

pub(super) struct CameraUi {
    pub state: CameraState,
    pub setup: Option<setup::Setup>,
    commands: mpsc::Sender<Command>,
}

impl CameraUi {
    pub(super) fn clear_setup(&mut self, cx: &mut gpui_kit::App) {
        if let Some(mut setup) = self.setup.take() {
            setup.stop_preview(None, cx);
        }
    }
}

pub(super) struct Worker {
    state: watch::Sender<CameraState>,
    commands: mpsc::Receiver<Command>,
}

enum Command {
    Start(CameraOptions),
    Stop,
}

pub(super) fn channel() -> (CameraUi, Worker, watch::Receiver<CameraState>) {
    let (send, commands) = mpsc::channel(2);
    let (state, updates) = watch::channel(CameraState::Idle);
    (
        CameraUi {
            state: CameraState::Idle,
            setup: None,
            commands: send,
        },
        Worker { state, commands },
        updates,
    )
}

pub(super) async fn run(room: &VoiceSession, mut worker: Worker) {
    let mut state = room.subscribe_camera();
    loop {
        tokio::select! {
            biased;
            command = worker.commands.recv() => {
                let result = match command {
                    Some(Command::Start(options)) => room.start_camera(options).await,
                    Some(Command::Stop) => room.stop_camera().await,
                    None => break,
                };
                if let Err(error) = result {
                    // Discard an older transport update before forwarding a command error.
                    state.borrow_and_update();
                    worker.state.send_replace(CameraState::Failed(error));
                }
            }
            changed = state.changed() => {
                if changed.is_err() { break; }
                worker.state.send_replace(*state.borrow_and_update());
            }
        }
    }
}

impl Voice {
    pub(super) fn is_publishing_camera(&self) -> bool {
        self.call
            .as_ref()
            .is_some_and(|call| call.camera.state == CameraState::Publishing)
    }
}
