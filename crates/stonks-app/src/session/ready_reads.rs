//! Ready gives message IDs, but not the type of the latest message.
use super::{spawn_core, ReadRequest, Session};
use futures_channel::mpsc::unbounded;
use futures_util::{
    future::{AbortHandle, Abortable},
    StreamExt,
};
use gpui_kit::Context;
use stonks_core::messages::{models::MessageQuery, read, service};

impl Session {
    pub(super) fn probe_ready_read_heads(&mut self, cx: &mut Context<Self>) {
        self.ready_read_probe = None;
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let heads: Vec<_> = snapshot
            .channels
            .iter()
            .filter_map(|channel| {
                let last = channel.last_message_id.as_ref()?;
                (channel.server.is_some()
                    && self.can_view_channel(&channel.id)
                    && read::cursor(snapshot, &channel.id).is_none_or(|seen| last.as_str() > seen))
                .then(|| (channel.id.clone(), last.clone()))
            })
            .collect();
        if heads.is_empty() {
            return;
        }
        let (abort, registration) = AbortHandle::new_pair();
        let policy = self.policy.clone();
        let task = cx.spawn(async move |this, cx| {
            let (tx, mut rx) = unbounded();
            let fetches = spawn_core(Abortable::new(
                async move {
                    futures_util::stream::iter(heads)
                        .for_each_concurrent(4, |(id, last)| {
                            let policy = &policy;
                            let tx = &tx;
                            async move {
                                let query = MessageQuery {
                                    channel_id: id.clone(),
                                    before: None,
                                    nearby: None,
                                };
                                if let Ok(page) = service::fetch_messages(policy, &query).await {
                                    let ids = page
                                        .messages
                                        .iter()
                                        .map(|m| (m.id.as_str(), m.system_type.as_deref()));
                                    if let Some(previous) = previous_readable_head(&last, ids) {
                                        let _ = tx.unbounded_send((
                                            id,
                                            last,
                                            previous.map(str::to_owned),
                                        ));
                                    }
                                }
                            }
                        })
                        .await;
                },
                registration,
            ));
            // The core runtime drives fetches; apply each result without waiting for the slowest channel.
            while let Some((id, last, previous)) = rx.next().await {
                if this
                    .update(cx, |this, cx| {
                        let Some(snapshot) = this.snapshot.as_mut() else {
                            return;
                        };
                        let seen = read::cursor(snapshot, &id).map(str::to_owned);
                        if let Some(channel) = snapshot.channels.iter_mut().find(|channel| {
                            channel.id == id
                                && channel.last_message_id.as_deref() == Some(last.as_str())
                        }) {
                            channel.last_message_id = previous.max(seen);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
            let _ = fetches.await;
        });
        self.ready_read_probe = Some(ReadRequest { abort, _task: task });
    }
}

// Only replace the advertised head when history confirms it is the hidden row.
fn previous_readable_head<'a>(
    last: &str,
    messages: impl Iterator<Item = (&'a str, Option<&'a str>)>,
) -> Option<Option<&'a str>> {
    let mut latest = None;
    let mut previous = None;
    for (id, kind) in messages {
        if latest.is_none_or(|(old, _)| id > old) {
            latest = Some((id, kind));
        }
        if id < last && kind != Some("call_started") {
            previous = previous.max(Some(id));
        }
    }
    (latest == Some((last, Some("call_started")))).then_some(previous)
}

#[cfg(test)]
mod tests {
    use super::previous_readable_head;

    #[test]
    fn ready_call_head_preserves_earlier_readable_message() {
        let a = "01KABCD0000000000000000001";
        let b = "01KABCD0000000000000000002";
        let c = "01KABCD0000000000000000003";
        assert_eq!(
            previous_readable_head(
                c,
                [
                    (a, None),
                    (c, Some("call_started")),
                    (b, Some("call_started"))
                ]
                .into_iter()
            ),
            Some(Some(a))
        );
        assert_eq!(
            previous_readable_head(c, [(c, Some("call_started"))].into_iter()),
            Some(None)
        );
        assert_eq!(
            previous_readable_head(b, [(c, None), (b, Some("call_started"))].into_iter()),
            None
        );
        assert_eq!(previous_readable_head(c, [(c, None)].into_iter()), None);
    }
}
