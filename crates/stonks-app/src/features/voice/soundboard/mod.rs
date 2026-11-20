//! Local clip library and the voice controller's bounded playback request.
mod quick;
mod settings;
use super::{cues, Voice};
use crate::{preferences::NativePreferences, runtime::spawn_core};
use gpui_kit::{
    component::{
        input::{InputEvent, InputState},
        slider::{SliderEvent, SliderState, SliderValue},
    },
    prelude::FluentBuilder,
    *,
};
use std::future::Future;
use std::path::{Path, PathBuf};
use stonks_core::{
    soundboard::{SoundboardClip, SoundboardError as Error, SoundboardStore},
    voice::{VoicePhase, VoiceSession},
};
use stonks_ui::{
    components::{Button, ButtonVariant, Feedback, FeedbackTone, SettingsInput, UiRange},
    i18n::{t, t_vars},
    icons::Icon,
    Theme,
};
use tokio::sync::oneshot;

pub(super) fn store(preferences: &Path) -> Option<SoundboardStore> {
    Some(SoundboardStore::new(
        &cues::app_data_directory()?,
        preferences.parent()?,
    ))
}

pub(super) struct Play {
    id: String,
    reply: oneshot::Sender<Result<(), Error>>,
}

impl Play {
    pub(super) fn reject_busy(self) {
        let _ = self.reply.send(Err(Error::Busy));
    }

    pub(super) async fn run(self, room: &VoiceSession, store: Option<&SoundboardStore>) {
        let Self { id, mut reply } = self;
        let result = tokio::select! {
            _ = reply.closed() => return,
            result = async {
                room.play_soundboard_clip(store.ok_or(Error::Storage)?, &id).await
            } => result,
        };
        let _ = reply.send(result);
    }
}

struct Library {
    clips: Vec<SoundboardClip>,
    join_clip_id: Option<String>,
}

enum Edit {
    Rename(String),
    Delete {
        clip: SoundboardClip,
        focus: FocusHandle,
    },
}

enum Change {
    Refresh,
    Import(PathBuf),
    Rename(String, String),
    Delete(String),
    Join(Option<String>),
}

impl Change {
    fn apply(self, store: SoundboardStore) -> Result<Library, Error> {
        match self {
            Self::Refresh => (),
            Self::Import(path) => {
                store.import(&path)?;
            }
            Self::Rename(id, name) => {
                store.rename(&id, &name)?;
            }
            Self::Delete(id) => {
                store.delete(&id)?;
            }
            Self::Join(id) => store.set_join_clip(id.as_deref())?,
        }
        Ok(Library {
            clips: store.list()?,
            join_clip_id: store.join_clip_id()?,
        })
    }
}

pub(super) struct Panel {
    library: Option<Library>,
    error: Option<String>,
    work: Option<Task<()>>,
    playback: Option<Task<()>>,
    edit: Option<Edit>,
    name: Entity<InputState>,
    listening: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
    quick: bool,
    anchor: Option<Bounds<Pixels>>,
    opener: Option<FocusHandle>,
}

impl Voice {
    fn soundboard_volume_control(&self, panel: &Panel, theme: Theme) -> impl IntoElement {
        div()
            .flex_shrink_0()
            .text_size(px(theme.font_size_xs))
            .text_color(theme.text_muted)
            .child(format!(
                "{}: {}%",
                t("voice.soundboardVolume"),
                self.listening.gains.soundboard_listening_gain()
            ))
            .child(UiRange::new(&panel.listening, theme).label(t("voice.soundboardVolume")))
            .when(self.listening.storage_error, |row| {
                row.child(Feedback::new(
                    theme,
                    FeedbackTone::Error,
                    t("voice.listeningSaveError"),
                ))
            })
    }

    pub(crate) fn soundboard_open(&self) -> bool {
        self.soundboard.is_some()
    }

    pub(super) fn quick_sounds_open(&self) -> bool {
        self.soundboard.as_ref().is_some_and(|panel| panel.quick)
    }

    pub(super) fn measure_sounds_anchor(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if let Some(panel) = self.soundboard.as_mut().filter(|panel| panel.quick) {
            if panel.anchor != Some(bounds) {
                panel.anchor = Some(bounds);
                cx.notify();
            }
        }
    }

    pub(crate) fn open_soundboard_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_soundboard(window, cx);
        self.open_soundboard_panel(false, window, cx);
    }

    pub(super) fn toggle_quick_sounds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_sounds_open() {
            self.close_soundboard(window, cx);
        } else if !self.soundboard_open() {
            self.open_soundboard_panel(true, window, cx);
        }
    }

    fn open_soundboard_panel(&mut self, quick: bool, window: &mut Window, cx: &mut Context<Self>) {
        let opener = window.focused(cx);
        let name = cx.new(|cx| {
            let input = InputState::new(window, cx);
            if quick {
                input.placeholder(t("voice.searchSounds"))
            } else {
                input
            }
        });
        if quick {
            name.update(cx, |input, cx| input.focus(window, cx));
        }
        self.participant_volume = None;
        let listening_gain = self.listening.gains.soundboard_listening_gain();
        let listening = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(f32::from(listening_gain))
        });
        let subscriptions = vec![
            cx.subscribe(&listening, |this, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(SliderValue::Single(value)) = event {
                    this.set_soundboard_gain(value.round() as u8, cx);
                }
            }),
            cx.subscribe_in(
                &name,
                window,
                move |this, _, event: &InputEvent, window, cx| {
                    if quick && matches!(event, InputEvent::Change) {
                        cx.notify();
                    } else if !quick && matches!(event, InputEvent::PressEnter { .. }) {
                        this.save_sound_name(window, cx);
                    }
                },
            ),
        ];
        self.soundboard = Some(Panel {
            library: None,
            error: None,
            work: None,
            playback: None,
            edit: None,
            name,
            listening,
            _subscriptions: subscriptions,
            opener,
            quick,
            anchor: None,
        });
        self.change(Change::Refresh, window, cx);
    }

    pub(crate) fn dismiss_soundboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = &mut self.soundboard {
            if !panel.quick && panel.work.is_some() {
                return;
            }
            if panel.edit.take().is_some() {
                if let Some(opener) = &panel.opener {
                    opener.focus(window, cx);
                }
                cx.notify();
                return;
            }
        }
        self.close_soundboard(window, cx);
    }

    fn close_soundboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = self.soundboard.take() {
            if let Some(opener) = panel.opener {
                opener.focus(window, cx);
            }
            cx.notify();
        }
    }

    fn save_sound_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = &self.soundboard {
            if let Some(Edit::Rename(id)) = &panel.edit {
                self.change(
                    Change::Rename(id.clone(), panel.name.read(cx).value().to_string()),
                    window,
                    cx,
                );
            }
        }
    }

    fn change(&mut self, change: Change, window: &mut Window, cx: &mut Context<Self>) {
        self.change_after(async move { Ok(Some(change)) }, window, cx);
    }

    fn change_after(
        &mut self,
        select: impl Future<Output = Result<Option<Change>, Error>> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let store = store(&cx.global::<NativePreferences>().path);
        let Some(panel) = &mut self.soundboard else {
            return;
        };
        if panel.work.is_some() {
            return;
        }
        panel.error = None;
        panel.work = Some(cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let Some(change) = select.await? else {
                    return Ok(None);
                };
                let store = store.ok_or(Error::Storage)?;
                spawn_core(async move {
                    tokio::task::spawn_blocking(move || change.apply(store))
                        .await
                        .map_err(|_| Error::Storage)?
                })
                .await
                .map(Some)
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(panel) = &mut this.soundboard else {
                    return;
                };
                panel.work = None;
                match result {
                    Ok(Some(library)) => {
                        panel.library = Some(library);
                        if panel.edit.take().is_some() {
                            if let Some(opener) = &panel.opener {
                                opener.focus(window, cx);
                            }
                        }
                    }
                    Ok(None) => (),
                    Err(error) => panel.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("soundboard.addSound").into()),
        });
        self.change_after(
            async move {
                match selection.await {
                    Ok(Ok(paths)) => Ok(paths
                        .and_then(|paths| paths.into_iter().next())
                        .map(Change::Import)),
                    _ => Err(Error::Storage),
                }
            },
            window,
            cx,
        );
    }

    fn can_play_sound(&self) -> bool {
        self.roster_state().is_some_and(|state| {
            state.phase == VoicePhase::Connected && state.can_speak && !state.deafened
        })
    }

    fn play_sound(&mut self, id: String, preview: bool, cx: &mut Context<Self>) {
        if !preview && !self.can_play_sound() {
            return;
        }
        let store = store(&cx.global::<NativePreferences>().path);
        let Some(panel) = &mut self.soundboard else {
            return;
        };
        if preview && panel.playback.is_some() {
            return;
        }
        let (mut reply, result) = oneshot::channel();
        if preview {
            let gains = self.listening.gains.clone();
            // Tie the core preview to the panel's receiver. Closing it drops the audio stream.
            let _preview = spawn_core(async move {
                let outcome = tokio::select! {
                    _ = reply.closed() => return,
                    outcome = async {
                        gains.preview_soundboard_clip(&store.ok_or(Error::Storage)?, &id, 100, "default").await
                    } => outcome,
                };
                let _ = reply.send(outcome);
            });
        } else if !self
            .call
            .as_ref()
            .is_some_and(|call| call.sounds.try_send(Play { id, reply }).is_ok())
        {
            panel.error = Some(Error::Busy.to_string());
            cx.notify();
            return;
        }
        panel.error = None;
        let panel_id = panel.name.entity_id();
        let task = cx.spawn(async move |this, cx| {
            let outcome = result.await.unwrap_or(Err(Error::Disconnected));
            let _ = this.update(cx, |this, cx| {
                if let Some(panel) = this
                    .soundboard
                    .as_mut()
                    .filter(|panel| panel.name.entity_id() == panel_id)
                {
                    if preview {
                        panel.playback = None;
                    }
                    if let Err(error) = outcome {
                        panel.error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        });
        if preview {
            panel.playback = Some(task);
        } else {
            // Closing the quick picker does not cancel a room clip.
            task.detach();
        }
        cx.notify();
    }

    pub(crate) fn soundboard_surface(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let panel = self.soundboard.as_ref().filter(|panel| panel.quick)?;
        self.quick_soundboard(panel, window, cx)
    }

    pub(crate) fn soundboard_settings_status(&self, cx: &App) -> (bool, bool, bool) {
        let Some(panel) = self.soundboard.as_ref().filter(|panel| !panel.quick) else {
            return (false, false, false);
        };
        let dirty = match &panel.edit {
            Some(Edit::Rename(id)) => panel
                .library
                .as_ref()
                .and_then(|library| library.clips.iter().find(|clip| &clip.id == id))
                .is_some_and(|clip| panel.name.read(cx).value().as_ref() != clip.name),
            _ => false,
        };
        (
            panel.work.is_some(),
            dirty,
            matches!(panel.edit, Some(Edit::Delete { .. })),
        )
    }

    pub(crate) fn close_soundboard_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.soundboard.as_ref().is_some_and(|panel| !panel.quick) {
            self.close_soundboard(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Change, Error, SoundboardStore};

    #[test]
    fn panel_updates_join_selection_without_touching_legacy_volume() {
        let dir = tempfile::tempdir().unwrap();
        let store = SoundboardStore::new(&dir.path().join("data"), &dir.path().join("config"));
        assert!(matches!(
            Change::Join(Some("missing".into())).apply(store.clone()),
            Err(Error::NotFound)
        ));
        store.set_volume(65).unwrap();
        let refreshed = Change::Join(None).apply(store.clone()).unwrap();
        assert_eq!(refreshed.join_clip_id, None);
        assert_eq!(store.settings().unwrap().soundboard_volume, 65);
    }
}
