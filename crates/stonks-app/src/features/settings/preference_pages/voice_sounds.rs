use super::{disclosure, State};
use gpui_kit::*;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use stonks_core::preferences::voice_sounds::{Cue, Error, Store};
use stonks_ui::{
    components::{Button, ButtonVariant, Feedback, FeedbackTone, UiAccordion},
    i18n::{t, t_vars},
    icons::Icon,
    Theme,
};

#[derive(Default)]
pub(super) struct VoiceSounds {
    open: bool,
    started: bool,
    loaded: bool,
    picking: bool,
    busy: HashSet<Cue>,
    overrides: HashMap<Cue, String>,
    notice: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Copy)]
enum Action {
    Preview,
    Change,
    Reset,
}

impl Action {
    fn key(self) -> &'static str {
        match self {
            Self::Preview => "settings.voiceSounds.preview",
            Self::Change => "settings.voiceSounds.change",
            Self::Reset => "settings.voiceSounds.reset",
        }
    }
}

fn label(cue: Cue) -> String {
    let suffix = match cue {
        Cue::Mute => "mute",
        Cue::Unmute => "unmute",
        Cue::Deafen => "deafen",
        Cue::Undeafen => "undeafen",
        Cue::Join => "join",
        Cue::Leave => "leave",
        Cue::Message => "message",
        Cue::StreamStart => "streamStart",
        Cue::StreamStop => "streamStop",
        Cue::ViewerJoin => "viewerJoin",
        Cue::ViewerLeave => "viewerLeave",
    };
    t(&format!("settings.voiceSounds.{suffix}"))
}

impl VoiceSounds {
    pub(super) fn render(&mut self, theme: &Theme, cx: &mut Context<State>) -> Div {
        if !self.started {
            self.started = true;
            let load = cx.background_spawn(async { Store::local()?.list() });
            cx.spawn(async move |this, cx| {
                let result = load.await;
                let _ = this.update(cx, |state, cx| {
                    let sounds = &mut state.voice_sounds;
                    match result {
                        Ok(overrides) => {
                            sounds.loaded = true;
                            sounds.overrides = overrides;
                        }
                        Err(error) => sounds.error = Some(error.to_string()),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        let subtitle = if self.overrides.is_empty() {
            t("settings.voiceSounds.shipped")
        } else {
            t_vars(
                "settings.voiceSounds.customized",
                &[("count", self.overrides.len()), ("total", Cue::ALL.len())],
            )
        };
        let weak = cx.entity().downgrade();
        let mut accordion =
            UiAccordion::new("voice-sounds", *theme, t("settings.voiceSounds.title"))
                .subtitle(subtitle)
                .leading(disclosure::icon(theme, Icon::Music))
                .open(self.open)
                .on_toggle(move |_, _, cx| {
                    let _ = weak.update(cx, |state, cx| {
                        state.voice_sounds.open = !state.voice_sounds.open;
                        cx.notify();
                    });
                });
        if self.open {
            let list = Cue::ALL.into_iter().fold(
                div().flex().flex_col().gap(px(theme.space2)),
                |list, cue| list.child(self.row(cue, theme, cx)),
            );
            accordion = accordion.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme.space1))
                    .child(
                        div()
                            .mx(px(theme.space1))
                            .text_color(theme.text_muted)
                            .text_size(px(theme.font_size_xs))
                            .child(t("settings.voiceSounds.note")),
                    )
                    .child(list),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(theme.space2))
            .children(
                self.error
                    .as_ref()
                    .map(|error| Feedback::new(*theme, FeedbackTone::Error, error.clone())),
            )
            .children(
                self.notice
                    .as_ref()
                    .map(|notice| Feedback::new(*theme, FeedbackTone::Success, notice.clone())),
            )
            .child(accordion)
    }

    fn row(&self, cue: Cue, theme: &Theme, cx: &Context<State>) -> Div {
        let custom = self.overrides.get(&cue);
        let file_key = if custom.is_some() {
            "settings.voiceSounds.custom"
        } else {
            "settings.voiceSounds.default"
        };
        let file = t_vars(
            file_key,
            &[(
                "file",
                custom.map(String::as_str).unwrap_or(cue.default_file()),
            )],
        );
        let mut actions = div()
            .flex()
            .flex_wrap()
            .min_w_0()
            .ml_auto()
            .gap(px(theme.space2));
        for action in [Action::Preview, Action::Change, Action::Reset] {
            if matches!(action, Action::Reset) && custom.is_none() {
                continue;
            }
            let weak = cx.entity().downgrade();
            actions = actions.child(
                Button::new(format!("{}-{}", cue.id(), action.key()), *theme)
                    .variant(ButtonVariant::Secondary)
                    .muted(matches!(action, Action::Reset))
                    .label(t(action.key()))
                    .disabled(
                        !self.loaded
                            || self.busy.contains(&cue)
                            || (self.picking && matches!(action, Action::Change)),
                    )
                    .on_click(move |_, _, cx| {
                        let _ =
                            weak.update(cx, |state, cx| state.voice_sounds.run(cue, action, cx));
                    }),
            );
        }
        div()
            .flex()
            .flex_wrap()
            .min_w_0()
            .min_h(px(58.))
            .items_center()
            .justify_between()
            .gap(px(theme.space3))
            .px(px(theme.space3))
            .py(px(theme.space2))
            .rounded(px(theme.radius_sm))
            .bg(theme.surface)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_grow(1.)
                    .flex_shrink(1.)
                    .flex_basis(px(160.))
                    .min_w_0()
                    .gap(px(theme.space1))
                    .child(
                        div()
                            .text_size(px(theme.font_size_md))
                            .font_weight(FontWeight::BOLD)
                            .child(label(cue)),
                    )
                    .child(
                        div()
                            .max_w(px(280.))
                            .truncate()
                            .text_size(px(theme.font_size_xs))
                            .text_color(theme.text_muted)
                            .child(file),
                    ),
            )
            .child(actions)
    }

    fn run(&mut self, cue: Cue, action: Action, cx: &mut Context<State>) {
        if !self.loaded
            || self.busy.contains(&cue)
            || (self.picking && matches!(action, Action::Change))
        {
            return;
        }
        self.busy.insert(cue);
        self.error = None;
        self.notice = None;
        let selection = if matches!(action, Action::Change) {
            self.picking = true;
            Some(cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some(t(action.key()).into()),
            }))
        } else {
            None
        };
        cx.notify();
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let selected = match selection {
                Some(selection) => {
                    let selected = match selection.await {
                        Ok(Ok(paths)) => Ok(paths.and_then(|paths| paths.into_iter().next())),
                        _ => Err(Error::Unavailable),
                    };
                    if this
                        .update(cx, |state, cx| {
                            state.voice_sounds.picking = false;
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                    selected
                }
                None => Ok(None),
            };
            let updated = match selected {
                Ok(path) => {
                    background
                        .spawn(async move { edit(cue, action, path) })
                        .await
                }
                Err(error) => Err(error),
            };
            let play = matches!(updated, Ok(Some(_))) || matches!(action, Action::Preview);
            if this
                .update(cx, |state, cx| {
                    let sounds = &mut state.voice_sounds;
                    match updated {
                        Ok(Some(Some(name))) => {
                            sounds.overrides.insert(cue, name);
                            sounds.notice = Some(t_vars(
                                "settings.voiceSounds.changed",
                                &[("sound", label(cue))],
                            ));
                        }
                        Ok(Some(None)) => {
                            sounds.overrides.remove(&cue);
                            sounds.notice = Some(t_vars(
                                "settings.voiceSounds.resetNotice",
                                &[("sound", label(cue))],
                            ));
                        }
                        Ok(None) => (),
                        Err(error) => sounds.error = Some(error.to_string()),
                    }
                    if !play {
                        sounds.busy.remove(&cue);
                    }
                    cx.notify();
                })
                .is_err()
                || !play
            {
                return;
            }
            let result = background
                .spawn(async move { Store::local()?.preview(cue) })
                .await;
            let _ = this.update(cx, |state, cx| {
                let sounds = &mut state.voice_sounds;
                sounds.busy.remove(&cue);
                if let Err(error) = result {
                    sounds.error = Some(error.to_string());
                }
                cx.notify();
            });
        })
        .detach();
    }
}

// Outer Option indicates an edit; inner Option is the new override name.
fn edit(cue: Cue, action: Action, path: Option<PathBuf>) -> Result<Option<Option<String>>, Error> {
    match (action, path) {
        (Action::Change, Some(path)) => Store::local()?
            .import(cue, &path)
            .map(|name| Some(Some(name))),
        (Action::Reset, _) => Store::local()?.reset(cue).map(|()| Some(None)),
        _ => Ok(None),
    }
}
