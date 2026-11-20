mod setup;

use super::{cues::Cue, Voice};
use stonks_core::voice::{
    screen::{ScreenShareError, ScreenShareOptions, ScreenShareState},
    VoicePhase, VoiceSession,
};
use tokio::sync::{mpsc, watch};

pub(super) struct ScreenUi {
    pub state: ScreenShareState,
    pub sharing: mpsc::Sender<Share>,
    pub setup: Option<setup::Setup>,
}

pub(super) struct ScreenWorker {
    pub state: watch::Sender<ScreenShareState>,
    pub sharing: mpsc::Receiver<Share>,
}

pub(super) enum Share {
    Start(ScreenShareOptions),
    Stop,
}

impl Voice {
    pub(super) fn is_sharing_screen(&self) -> bool {
        self.call
            .as_ref()
            .is_some_and(|call| call.screen.state == ScreenShareState::Sharing)
    }
}

pub(super) async fn run(room: &VoiceSession, worker: ScreenWorker, cues: &mpsc::Sender<Cue>) {
    share(room, worker.state, worker.sharing, cues).await;
}

async fn share(
    room: &VoiceSession,
    output: watch::Sender<ScreenShareState>,
    mut commands: mpsc::Receiver<Share>,
    cues: &mpsc::Sender<Cue>,
) {
    use ScreenShareState::*;
    let mut state = room.subscribe_screen_share();
    let mut published = false;
    loop {
        tokio::select! {
            biased;
            changed = state.changed() => {
                if changed.is_err() { break; }
                let current = *state.borrow_and_update();
                let next = current == Sharing || (current == Stopping && published);
                if next != published {
                    let _ = cues.try_send(if next { Cue::ShareStart } else { Cue::ShareStop });
                    published = next;
                }
                output.send_replace(if current == Failed(ScreenShareError::Cancelled) { Idle } else { current });
            }
            command = commands.recv() => {
                let Some(command) = command else { break };
                let result = match command {
                    Share::Start(options) => room.start_screen_share_with_options(options).await,
                    Share::Stop => room.stop_screen_share().await,
                };
                if let Err(error) = result {
                    output.send_replace(if error == ScreenShareError::Cancelled { Idle } else { Failed(error) });
                }
            }
        }
    }
}
