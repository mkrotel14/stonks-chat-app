//! Same readable custom token and wire serialization rules as customEmojis.ts.
use stonks_ui::components::CustomEmoji;

pub fn display_token(emoji: &CustomEmoji, emojis: &[CustomEmoji]) -> String {
    let duplicate = emojis
        .iter()
        .any(|other| other.id != emoji.id && other.name.eq_ignore_ascii_case(&emoji.name));
    format!(":{}: ", if duplicate { &emoji.id } else { &emoji.name })
}
pub fn serialize(content: &str, emojis: &[CustomEmoji]) -> String {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find(':') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest[1..].find(':').map(|index| index + 1) else {
            break;
        };
        let name = &rest[1..end];
        let valid = !name.is_empty()
            && name.len() <= 32
            && name.bytes().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, b'_' | b'-')
            });
        if valid {
            if let Some(emoji) = emojis
                .iter()
                .find(|emoji| emoji.name.eq_ignore_ascii_case(name))
            {
                result.push(':');
                result.push_str(&emoji.id);
                result.push(':');
            } else {
                result.push_str(&rest[..=end]);
            }
            rest = &rest[end + 1..];
        } else {
            result.push(':');
            rest = &rest[1..];
        }
    }
    result.push_str(rest);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    fn emoji(id: &str, name: &str) -> CustomEmoji {
        CustomEmoji {
            id: id.into(),
            name: name.into(),
            server_name: None,
            url: None,
        }
    }
    #[test]
    fn readable_tokens_serialize_without_corrupting_unicode_or_unknown_names() {
        let emojis = vec![emoji("01M00000000000000000000000", "wave")];
        assert_eq!(display_token(&emojis[0], &emojis), ":wave: ");
        assert_eq!(
            serialize("Olá 👋 :WaVe: :unknown: :not a token: 🇧🇷", &emojis),
            "Olá 👋 :01M00000000000000000000000: :unknown: :not a token: 🇧🇷"
        );
    }
    #[test]
    fn duplicate_names_insert_unambiguous_wire_id() {
        let emojis = vec![
            emoji("01M00000000000000000000000", "wave"),
            emoji("01M00000000000000000000001", "WAVE"),
        ];
        let token = display_token(&emojis[1], &emojis);
        assert_eq!(token, ":01M00000000000000000000001: ");
        assert_eq!(serialize(&token, &emojis), token);
        assert_eq!(
            serialize(":wave: :wave:", &emojis),
            ":01M00000000000000000000000: :01M00000000000000000000000:"
        );
    }
}
