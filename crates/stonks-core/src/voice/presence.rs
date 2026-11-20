use std::collections::HashSet;

/// An audible remote membership change in the joined LiveKit room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoicePresence {
    Joined,
    Left,
}

#[derive(Default)]
pub(super) struct Presence {
    members: HashSet<String>,
    live: bool,
}

impl Presence {
    pub(super) fn seed(&mut self, members: impl IntoIterator<Item = String>, local_id: &str) {
        self.members = members.into_iter().filter(|id| id != local_id).collect();
        self.set_live(true);
    }

    pub(super) fn set_live(&mut self, live: bool) {
        self.live = live;
    }

    pub(super) fn update(
        &mut self,
        id: String,
        local_id: &str,
        joined: bool,
    ) -> Option<VoicePresence> {
        if id == local_id {
            return None;
        }
        // Keep the baseline current even while LiveKit reconciles a reconnect.
        let (changed, event) = if joined {
            (self.members.insert(id), VoicePresence::Joined)
        } else {
            (self.members.remove(&id), VoicePresence::Left)
        };
        (self.live && changed).then_some(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_roster_self_and_duplicates_are_silent() {
        let mut presence = Presence::default();
        assert_eq!(presence.update("early".into(), "me", true), None);
        presence.seed(["existing".into(), "me".into()], "me");
        for (id, joined, expected) in [
            ("existing", true, None),
            ("me", true, None),
            ("me", false, None),
            ("existing", false, Some(VoicePresence::Left)),
            ("existing", false, None),
            ("new", true, Some(VoicePresence::Joined)),
            ("new", true, None),
            ("new", false, Some(VoicePresence::Left)),
            ("new", true, Some(VoicePresence::Joined)),
        ] {
            assert_eq!(presence.update(id.into(), "me", joined), expected);
        }
    }

    #[test]
    fn reconnect_updates_membership_silently_and_resumes_live_edges() {
        let mut presence = Presence::default();
        presence.seed(["departed".into(), "stayed".into()], "me");
        presence.set_live(false);
        assert_eq!(presence.update("departed".into(), "me", false), None);
        assert_eq!(presence.update("arrived".into(), "me", true), None);
        presence.set_live(true);
        assert_eq!(presence.update("arrived".into(), "me", true), None);
        assert_eq!(presence.update("departed".into(), "me", false), None);
        assert_eq!(presence.update("stayed".into(), "me", true), None);
        assert_eq!(
            presence.update("arrived".into(), "me", false),
            Some(VoicePresence::Left)
        );
        assert_eq!(
            presence.update("departed".into(), "me", true),
            Some(VoicePresence::Joined)
        );
    }
}
