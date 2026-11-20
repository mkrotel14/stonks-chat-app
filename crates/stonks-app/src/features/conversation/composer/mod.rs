//! Native message composer: shared state, host API, and editor event wiring.
use futures_util::future::AbortHandle;
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::{App, AppContext, Context, Entity, EventEmitter, Subscription, Task, Window};
use std::{collections::HashMap, rc::Rc, sync::Arc, time::Instant};
use stonks_core::{http::HttpPolicy, messages::models::TimelineMessage};

mod attachments;
mod editor;
mod emoji;
mod mentions;
mod send;
mod spellcheck;
mod state;
mod typing;
mod view;

use state::ComposerState;

use attachments::route::AttachmentRoute;
pub use attachments::{route::core_attachment_route, AttachmentAccess};
pub use typing::Typing;

struct Request {
    abort: AbortHandle,
    _task: Task<()>,
}
impl Drop for Request {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

pub(crate) struct MusicSubmission {
    pub result: tokio::sync::oneshot::Receiver<Result<(), String>>,
    pub is_current: Box<dyn Fn(&App) -> bool>,
}

type MusicRoute = Rc<dyn Fn(String, &mut App) -> Result<MusicSubmission, String>>;

pub struct Sent(pub TimelineMessage);

pub struct Composer {
    input: Entity<TextareaState>,
    state: ComposerState,
    policy: Arc<HttpPolicy>,
    requests: HashMap<u64, Request>,
    pub(crate) music_route: Option<MusicRoute>,
    attachment_route: Option<AttachmentRoute>,
    attachment_access: Option<AttachmentAccess>,
    picker: Option<Task<()>>,
    spellcheck: spellcheck::Spellcheck,
    typing: typing::TypingState,
    typing_clock: Instant,
    typing_timer: Option<Task<()>>,
    typing_enabled: bool,
    remote_typing: typing::RemoteTyping,
    remote_timer: Option<Task<()>>,
    channel_name: Option<String>,
    emoji: emoji::EmojiState,
    mentions: mentions::Mentions,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<Sent> for Composer {}
impl EventEmitter<Typing> for Composer {}

impl Composer {
    pub fn new(policy: Arc<HttpPolicy>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx));
        let picker = cx.new(|cx| stonks_ui::components::UiEmojiPicker::new(window, cx));
        let composer = cx.entity().downgrade();
        let subscriptions = vec![
            cx.intercept_keystrokes(move |event, window, cx| {
                let _ = composer.update(cx, |this, cx| {
                    this.mention_key(&event.keystroke, window, cx)
                });
            }),
            cx.observe(&input, |_, _, cx| cx.notify()),
            cx.subscribe_in(&picker, window, |this, _, event, window, cx| {
                this.select_emoji(event, window, cx)
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.stop_typing(cx);
                }
            }),
            cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                match event {
                    InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                    InputEvent::Change => this.changed(cx),
                    InputEvent::Blur => {
                        this.mentions.open = false;
                        this.stop_typing(cx);
                    }
                    _ => {}
                }
                cx.notify();
            }),
        ];
        Self {
            input,
            state: ComposerState::default(),
            policy,
            requests: HashMap::new(),
            music_route: None,
            attachment_route: None,
            attachment_access: None,
            picker: None,
            spellcheck: spellcheck::Spellcheck::default(),
            typing: typing::TypingState::default(),
            typing_clock: Instant::now(),
            typing_timer: None,
            typing_enabled: false,
            remote_typing: typing::RemoteTyping::default(),
            remote_timer: None,
            channel_name: None,
            emoji: emoji::EmojiState::new(picker, cx.focus_handle()),
            mentions: mentions::Mentions::default(),
            _subscriptions: subscriptions,
        }
    }
}
