//! Generations keep old requests from publishing into a replacement profile.
use stonks_core::{
    connection::ClientUser,
    member_actions::{relationship_kind, RelationshipKind},
};
pub(super) struct RelationshipRequest {
    user: String,
    kind: RelationshipKind,
}
impl RelationshipRequest {
    pub fn new(user: &ClientUser) -> Self {
        Self {
            user: user.id.clone(),
            kind: relationship_kind(user.relationship.as_deref()),
        }
    }
    pub fn accepts(&self, user: &ClientUser) -> bool {
        self.user == user.id && self.kind == relationship_kind(user.relationship.as_deref())
    }
}
#[derive(Default)]
pub(super) struct IdentityDraft {
    pub nickname: String,
    pub pronouns: String,
}
impl IdentityDraft {
    pub fn authorize(
        &self,
        nickname: &str,
        pronouns: &str,
        names_allowed: bool,
        avatar_selected: bool,
        avatar_allowed: bool,
    ) -> Result<(), &'static str> {
        if !names_allowed
            && (nickname.trim() != self.nickname.trim() || pronouns.trim() != self.pronouns.trim())
        {
            return Err("Your permission to edit this identity changed. Your draft has been kept.");
        }
        if avatar_selected && !avatar_allowed {
            return Err(
                "Your permission to change this avatar changed. Your selected image has been kept.",
            );
        }
        Ok(())
    }
}
pub(super) fn field_length(value: &str) -> usize {
    value.encode_utf16().count()
}
#[derive(Default)]
pub(super) struct Gate {
    generation: u64,
    serial: u64,
    busy: bool,
}
impl Gate {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn busy(&self) -> bool {
        self.busy
    }
    pub fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
    }
    pub fn start(&mut self) -> Option<(u64, u64)> {
        if self.busy {
            return None;
        }
        self.busy = true;
        self.serial = self.serial.wrapping_add(1);
        Some((self.generation, self.serial))
    }
    pub fn complete(&mut self, ticket: (u64, u64)) -> bool {
        if (self.generation, self.serial) != ticket || !self.busy {
            return false;
        }
        self.busy = false;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_and_logout_reject_stale_completion() {
        let mut gate = Gate::default();
        let old = gate.start().unwrap();
        assert!(gate.start().is_none());
        gate.reset();
        let new = gate.start().unwrap();
        assert!(!gate.complete(old));
        assert!(gate.busy());
        assert!(gate.complete(new));
        assert!(!gate.complete(new));
        let next = gate.start().unwrap();
        assert!(!gate.complete(new));
        assert!(gate.complete(next));
    }
    #[test]
    fn revoked_permissions_never_silently_discard_identity_or_avatar_drafts() {
        let draft = IdentityDraft {
            nickname: "Name".into(),
            pronouns: "they/them".into(),
        };
        assert!(draft
            .authorize("Changed", "they/them", false, false, false)
            .is_err());
        assert!(draft.authorize("Name", "new", false, false, false).is_err());
        assert!(draft
            .authorize("Name", "they/them", true, true, false)
            .is_err());
        assert!(draft
            .authorize("Name", "they/them", false, true, true)
            .is_ok());
        assert_eq!(field_length("a🙂"), 3);
    }
    #[test]
    fn old_relationship_response_cannot_replace_a_new_gateway_block() {
        let mut user: ClientUser = serde_json::from_value(
            serde_json::json!({"_id":"user","username":"Member","relationship":"None"}),
        )
        .unwrap();
        let request = RelationshipRequest::new(&user);
        user.online = true;
        assert!(request.accepts(&user));
        user.relationship = Some("no_relationship".into());
        assert!(request.accepts(&user));
        user.relationship = Some("Blocked".into());
        assert!(!request.accepts(&user));
        user.relationship = None;
        user.id = "someoneElse".into();
        assert!(!request.accepts(&user));
    }
    #[test]
    fn ticket_completion_does_not_release_or_publish_over_ownership_work() {
        let mut gate = Gate::default();
        let verification = gate.start().unwrap();
        assert!(gate.complete(verification));
        let ownership = gate.start().unwrap();
        assert!(gate.busy());
        assert!(!gate.complete(verification));
        assert!(gate.busy());
        gate.reset();
        assert!(!gate.complete(ownership));
    }
}
