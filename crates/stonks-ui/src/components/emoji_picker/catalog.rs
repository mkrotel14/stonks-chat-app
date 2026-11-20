//! Generated from the same installed emoji-datasource-twitter and filtering
//! rules as vite.config.ts::compactEmojiData and src/features/messages/emojis.tsx.
use std::sync::OnceLock;

pub struct Entry {
    pub emoji: String,
    pub label: String,
    pub category: String,
    pub search: String,
}
pub fn entries() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let rows: Vec<(String, String, String, String)> =
            serde_json::from_str(include_str!("../../../assets/emoji/picker.json"))
                .expect("bundled source emoji catalog is valid");
        rows.into_iter()
            .map(|(emoji, label, category, search)| Entry {
                emoji,
                label,
                category,
                search,
            })
            .collect()
    })
}
pub fn categories() -> Vec<String> {
    let mut categories: Vec<_> = entries()
        .iter()
        .map(|entry| entry.category.clone())
        .collect();
    categories.sort();
    categories.dedup();
    categories
}
pub fn matches(query: &str, category: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    entries()
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            (if query.is_empty() {
                entry.category == category
            } else {
                entry.search.contains(&query)
            })
            .then_some(index)
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_catalog_search_covers_names_shortcodes_tones_and_categories() {
        assert!(entries().len() > 3000);
        let smiles = matches("", "Smileys & Emotion");
        assert!(!smiles.is_empty());
        assert!(smiles
            .iter()
            .all(|&i| entries()[i].category == "Smileys & Emotion"));
        for query in ["thumbsup", "skin tone", "brazil", "family"] {
            assert!(!matches(query, "unused").is_empty(), "{query}");
        }
        assert_eq!(matches("  BrAZil ", "unused"), matches("brazil", "unused"));
        assert!(matches("no-emoji-has-this-phrase", "Smileys & Emotion").is_empty());
        assert_eq!(categories().first().map(String::as_str), Some("Activities"));
    }
}
