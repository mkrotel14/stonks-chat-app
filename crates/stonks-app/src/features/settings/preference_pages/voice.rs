use super::*;
use crate::runtime::spawn_core;
use gpui_kit::component::slider::{SliderEvent, SliderState, SliderValue};
use stonks_core::{
    preferences::VideoQuality,
    voice::devices::{self, Device, DiscoveryError, Kind},
};
use stonks_ui::{components::UiRange, icons::Icon};

#[derive(Default)]
pub(super) struct VoiceFields {
    devices: Option<Result<Vec<Device>, DiscoveryError>>,
    scan: Option<Task<()>>,
    fields: [disclosure::Disclosure; 6],
    ranges: Vec<(Entity<SliderState>, Subscription)>,
}

fn scan(cx: &mut Context<State>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let devices = spawn_core(devices::discover()).await;
        let _ = this.update(cx, |this, cx| {
            this.voice.devices = Some(devices);
            this.voice.scan = None;
            cx.notify();
        });
    })
}

fn key(name: &str) -> String {
    format!("settings.voice.{name}")
}

pub(super) fn render(
    state: &mut State,
    theme: &Theme,
    prefs: &Preferences,
    disabled: bool,
    window: &mut Window,
    cx: &mut Context<State>,
) -> Div {
    let voice = &mut state.voice;
    if voice.devices.is_none() && voice.scan.is_none() {
        voice.scan = Some(scan(cx));
    }
    if voice.ranges.is_empty() {
        for (is_input, volume) in [
            (true, prefs.microphone_volume),
            (false, prefs.output_volume),
        ] {
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(0.)
                    .max(200.)
                    .step(1.)
                    .default_value(f32::from(volume.min(200)))
            });
            let subscription =
                cx.subscribe_in(&slider, window, move |this, slider, event, window, cx| {
                    if matches!(event, SliderEvent::Change(_)) {
                        cx.notify();
                    }
                    // Avoid synchronous preference writes on every pointer move.
                    if let SliderEvent::Release(SliderValue::Single(value)) = event {
                        let value = value.round() as u8;
                        let saved = &cx.global::<NativePreferences>().saved;
                        let (prior, change) = if is_input {
                            (saved.microphone_volume, Change::MicrophoneVolume(value))
                        } else {
                            (saved.output_volume, Change::OutputVolume(value))
                        };
                        if value != prior && !this.save(change, window, cx) {
                            slider.update(cx, |slider, cx| {
                                slider.set_value(f32::from(prior.min(200)), window, cx)
                            });
                        }
                    }
                });
            voice.ranges.push((slider, subscription));
        }
    }
    let scanning = voice.scan.is_some();
    let notice = match &voice.devices {
        Some(Err(_)) => Some(t(&key("discoveryFailed"))),
        Some(Ok(devices)) if devices.is_empty() => Some(t(&key("noDevices"))),
        _ => None,
    };
    let mut volume = settings_section(theme, &t(&key("volume")));
    for ((slider, _), name) in voice.ranges.iter().zip(["microphone", "outputVolume"]) {
        let label = t(&key(name));
        let SliderValue::Single(value) = slider.read(cx).value() else {
            continue;
        };
        volume = volume.child(
            settings_row(
                theme,
                &label,
                &t(&key(&format!("{name}Description"))),
                div()
                    .w(relative(0.48))
                    .min_w(px(180.))
                    .max_w(px(290.))
                    .flex()
                    .items_center()
                    .gap(px(theme.space3))
                    .child(
                        div().flex_1().child(
                            UiRange::new(slider, *theme)
                                .label(label.clone())
                                .disabled(disabled),
                        ),
                    )
                    .child(
                        div()
                            .w(px(42.))
                            .flex_shrink_0()
                            .text_right()
                            .text_size(px(theme.font_size_xs))
                            .text_color(theme.text_muted)
                            .child(format!("{value:.0}%")),
                    ),
            )
            .min_h(px(66.)),
        );
    }
    let input = device_choices(
        &voice.devices,
        Kind::Input,
        &prefs.input_device,
        Change::InputDevice,
    );
    let output = device_choices(
        &voice.devices,
        Kind::Output,
        &prefs.output_device,
        Change::OutputDevice,
    );
    let camera = device_choices(
        &voice.devices,
        Kind::Camera,
        &prefs.video_device,
        Change::VideoDevice,
    );
    let camera_quality = quality_choices(prefs.camera_quality, Change::CameraQuality);
    let screen_quality = quality_choices(prefs.screen_share_quality, Change::ScreenShareQuality);
    let noise = (
        [("RNNoise", true), ("—", false)]
            .into_iter()
            .map(|(label, enabled)| disclosure::Choice {
                label: label.into(),
                description: None,
                change: Some(Change::NoiseSuppression(enabled)),
            })
            .collect(),
        usize::from(prefs.noise_suppression_mode == "disabled"),
    );
    type Access = fn(&mut State) -> &mut disclosure::Disclosure;
    let mut field = |access: Access, id, icon, (options, selected)| {
        disclosure::render(
            access(state),
            access,
            id,
            theme,
            t(&key(id)),
            Some(icon),
            options,
            selected,
            disabled,
            cx,
        )
    };
    let input = field(
        |state| &mut state.voice.fields[0],
        "inputDevice",
        Icon::Mic,
        input,
    );
    let output = field(
        |state| &mut state.voice.fields[1],
        "outputDevice",
        Icon::Volume2,
        output,
    );
    let camera = field(
        |state| &mut state.voice.fields[2],
        "videoDevice",
        Icon::CameraOutline,
        camera,
    );
    let camera_quality = field(
        |state| &mut state.voice.fields[3],
        "cameraQuality",
        Icon::CameraOutline,
        camera_quality,
    );
    let screen_quality = field(
        |state| &mut state.voice.fields[4],
        "screenQuality",
        Icon::MonitorUp,
        screen_quality,
    );
    let noise = field(
        |state| &mut state.voice.fields[5],
        "noiseSuppression",
        Icon::Volume2,
        noise,
    );
    let weak = cx.entity().downgrade();
    div()
        .child(
            div()
                .text_color(theme.text_muted)
                .text_size(px(theme.font_size_xs))
                .child(t(&key("nextJoin"))),
        )
        .child(
            settings_section(theme, &t(&key("devices")))
                .child(input)
                .child(output)
                .child(camera)
                .child(camera_quality)
                .children(notice.map(|notice| {
                    div()
                        .mt(px(theme.space1))
                        .mx(px(1.))
                        .text_size(px(theme.font_size_xs))
                        .text_color(theme.text_muted)
                        .child(notice)
                }))
                .child(
                    div().flex().mt(px(2.)).child(
                        Button::new("refresh-voice-devices", *theme)
                            .variant(ButtonVariant::Secondary)
                            .label(t(&key(if scanning { "scanning" } else { "refresh" })))
                            .disabled(scanning)
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    if this.voice.scan.is_none() {
                                        this.voice.scan = Some(scan(cx));
                                        cx.notify();
                                    }
                                });
                            }),
                    ),
                ),
        )
        .child(volume)
        .child(
            settings_section(theme, &t(&key("processing")))
                .child(noise)
                .child(toggle(
                    theme,
                    "settings.voice.echoCancellation",
                    prefs.echo_cancellation,
                    disabled,
                    Change::EchoCancellation,
                    cx,
                ))
                .child(toggle(
                    theme,
                    "settings.voice.gainControl",
                    prefs.automatic_gain_control,
                    disabled,
                    Change::AutomaticGainControl,
                    cx,
                )),
        )
        .child(
            settings_section(theme, &t(&key("screenSharing")))
                .child(screen_quality)
                .child(toggle(
                    theme,
                    "settings.voice.shareAudio",
                    prefs.screen_share_audio,
                    disabled,
                    Change::ScreenShareAudio,
                    cx,
                )),
        )
}

fn device_choices(
    devices: &Option<Result<Vec<Device>, DiscoveryError>>,
    kind: Kind,
    saved: &str,
    change: fn(String) -> Change,
) -> (Vec<disclosure::Choice>, usize) {
    let mut options = vec![disclosure::Choice {
        label: t(&key("systemDefault")),
        description: None,
        change: Some(change("default".into())),
    }];
    let mut selected = (saved == "default").then_some(0);
    if let Some(Ok(devices)) = devices {
        for device in devices.iter().filter(|device| device.kind == kind) {
            if device.id == saved {
                selected = Some(options.len());
            }
            options.push(disclosure::Choice {
                label: device.label.clone(),
                description: device.is_default.then(|| t(&key("currentDefault"))),
                change: Some(change(device.id.clone())),
            });
        }
    }
    let selected = selected.unwrap_or_else(|| {
        let index = options.len();
        options.push(disclosure::Choice {
            label: t(&key("unavailable")),
            description: None,
            change: None,
        });
        index
    });
    (options, selected)
}

fn quality_choices(
    saved: VideoQuality,
    change: fn(VideoQuality) -> Change,
) -> (Vec<disclosure::Choice>, usize) {
    (
        VideoQuality::ALL
            .into_iter()
            .map(|quality| disclosure::Choice {
                label: quality.camera().label().into(),
                description: None,
                change: Some(change(quality)),
            })
            .collect(),
        saved as usize,
    )
}
