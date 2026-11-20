use super::*;
impl MediaPlayer {
    pub(super) fn preload_audio_metadata(&mut self, cx: &mut Context<Self>) {
        if !self.enabled
            || self.spec.kind != MediaKind::Audio
            || self.metadata.is_some()
            || self.audio_duration > 0.
        {
            return;
        }
        let source = match stonks_core::media_playback::Source::new(
            self.spec.key.clone(),
            self.spec.url.clone(),
            MediaKind::Audio,
            false,
            Vec::new(),
        ) {
            Ok(source) => source,
            Err(_) => return,
        };
        let generation = self.metadata_generation;
        let (abort, registration) = AbortHandle::new_pair();
        let task = cx.spawn(async move |this, cx| {
            let Ok(result) = spawn_core(Abortable::new(
                stonks_core::media_playback::probe_audio(source),
                registration,
            ))
            .await
            else {
                return;
            };
            this.update(cx, |this, cx| {
                if !this.enabled || this.metadata_generation != generation {
                    return;
                }
                match result {
                    Ok(metadata) => {
                        this.audio_duration = metadata.duration;
                        if this.current.duration <= 0. {
                            this.current.duration = metadata.duration;
                        }
                        cx.notify();
                    }
                    // Metadata failure leaves Play available: the explicit
                    // playback attempt owns the source retry/error surface.
                    Err(_) => {}
                }
            })
            .ok();
        });
        self.metadata = Some(Work { abort, _task: task });
    }
}
