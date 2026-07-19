use crate::graphics::special_char::SpecialCharPosition;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Default, Clone)]
pub struct TagList {
    file_path: PathBuf,
    tags: Arc<HashMap<String, String>>,
    pattern: Arc<String>,
    autocomplete_list: Vec<Arc<String>>,
    selected: usize,
}

impl TagList {
    fn parse_tag_list(file_path: &Path) -> Result<HashMap<String, String>, String> {
        if !file_path.exists() {
            return Ok(HashMap::new());
        }

        let tag_file_content = std::fs::read_to_string(file_path)
            .map_err(|err| format!("Cannot read tag file at {}: {}", file_path.display(), err))?;
        serde_yaml::from_str(&tag_file_content).map_err(|err| {
            format!(
                "Failed to parse tag file at {}: {}",
                file_path.display(),
                err
            )
        })
    }

    /// The tag entry currently highlighted in the autocomplete pop-up, if any.
    pub fn get_selected_autocomplete(&self) -> Option<Arc<String>> {
        self.autocomplete_list.get(self.selected).cloned()
    }

    /// Index of the highlighted entry within the autocomplete list.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Move the highlight down one entry, stopping at the last item.
    pub fn select_next(&mut self) {
        if self.autocomplete_list.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.autocomplete_list.len() - 1);
    }

    /// Move the highlight up one entry, stopping at the first item.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Whether the autocomplete pop-up currently has entries to show.
    pub fn has_suggestions(&self) -> bool {
        !self.pattern.is_empty() && !self.autocomplete_list.is_empty()
    }

    pub fn new(file_path: PathBuf) -> Result<Self, String> {
        let tags = Self::parse_tag_list(&file_path)?;
        Ok(Self {
            file_path,
            tags: Arc::new(tags),
            ..Default::default()
        })
    }

    pub fn reload(&mut self) -> Result<(), String> {
        let tags = Self::parse_tag_list(&self.file_path)?;
        self.tags = Arc::new(tags);
        Ok(())
    }

    pub fn get_tagged_key(&self, key: &str) -> String {
        let tag_name = key.strip_prefix('@').unwrap_or_default();
        self.tags
            .get(tag_name)
            .cloned()
            .unwrap_or(format!("@{}", tag_name))
    }

    pub fn update_pattern(&mut self, text: &str, cursor: usize) {
        if text.is_empty() || cursor == 0 {
            self.pattern = Arc::new(String::new());
            return;
        }

        let Some(at_pos) = text
            .chars()
            .enumerate()
            .take(cursor)
            .filter_map(|(i, c)| (c == '@').then_some(i))
            .last()
        else {
            self.pattern = Arc::new(String::new());
            return;
        };

        if text
            .chars()
            .skip(at_pos)
            .take(cursor - at_pos)
            .any(|c| c.is_whitespace())
        {
            self.pattern = Arc::new(String::new());
            return;
        }

        self.pattern = Arc::new(
            text.chars()
                .skip(at_pos)
                .take(cursor - at_pos)
                .collect::<String>(),
        );
    }

    pub fn update_autocomplete_list(&mut self) {
        let pattern_rest = self.pattern.chars().skip(1).collect::<String>();

        self.autocomplete_list = self
            .tags
            .keys()
            .filter(|&k| match self.pattern.chars().count() {
                0 => false,
                1 if self.pattern.as_str() == "@" => true,
                1 if self.pattern.as_str() != "@" => false,
                _ => {
                    k != &pattern_rest
                        && self.pattern.starts_with('@')
                        && k.starts_with(&pattern_rest)
                }
            })
            .map(|k| Arc::new(k.clone()))
            .collect();
        self.autocomplete_list
            .sort_by_key(|a| a.to_ascii_lowercase());
        // The list is rebuilt on every keystroke, so the highlight returns to
        // the top; the user drives it away with the up/down arrows.
        self.selected = 0;
    }

    pub fn clear(&mut self) {
        self.pattern = Arc::new(String::new());
        self.autocomplete_list.clear();
        self.selected = 0;
    }

    pub fn full_clear(&mut self) {
        self.clear();
        self.tags = Arc::new(HashMap::new());
    }

    pub fn tag_filter(&self, string: &str) -> Option<SpecialCharPosition> {
        for (char_pos, _) in string.chars().enumerate().filter(|(_, c)| *c == '@') {
            let tag_name: String = string
                .chars()
                .skip(char_pos + 1)
                .take_while(|c| !c.is_whitespace() && *c != '@' && *c != '"')
                .collect();

            if tag_name.is_empty() {
                continue;
            }

            if self.tags.contains_key(&tag_name) {
                return Some((char_pos, tag_name.chars().count() + 1).into());
            }
        }

        None
    }

    pub fn autocomplete_list(&self) -> Vec<Arc<String>> {
        self.autocomplete_list.clone()
    }

    pub fn pattern(&self) -> Arc<String> {
        self.pattern.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::special_char::{SpecialCharItem, ToSpecialChar};

    fn tag_list_with(tags: &[(&str, &str)]) -> TagList {
        let map = tags
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<_, _>>();
        TagList {
            tags: Arc::new(map),
            ..Default::default()
        }
    }

    #[test]
    fn test_tag_filter_single() {
        let tag_list = tag_list_with(&[("tag1", "v1")]);
        let pos = tag_list.tag_filter("@tag1").expect("tag should match");
        assert_eq!(pos.start, 0);
        assert_eq!(pos.length, 5);
    }

    #[test]
    fn test_tag_filter_adjacent_no_sep() {
        let tag_list = tag_list_with(&[("tag1", "v1"), ("tag2", "v2")]);
        let pos = tag_list
            .tag_filter("@tag1@tag2")
            .expect("first tag should match");
        assert_eq!(pos.start, 0);
        assert_eq!(pos.length, 5);
    }

    #[test]
    fn test_tag_filter_adjacent_resolves_both() {
        let tag_list = tag_list_with(&[("tag1", "v1"), ("tag2", "v2")]);
        let content = "@tag1@tag2";
        let mut resolved = String::new();
        for item in content.to_special_char(|s| tag_list.tag_filter(s)) {
            match item {
                SpecialCharItem::Plain(s) => resolved.push_str(&s),
                SpecialCharItem::Special(s, _) => resolved.push_str(&tag_list.get_tagged_key(&s)),
            }
        }
        assert_eq!(resolved, "v1v2");
    }

    #[test]
    fn test_tag_filter_closing_quote_terminates_tag() {
        // Issue #186: a `"` right after a tag must delimit the name, so no
        // trailing space is needed to invoke a tag inside quotes.
        let tag_list = tag_list_with(&[("tag1", "v1")]);
        let pos = tag_list
            .tag_filter("\"@tag1\"")
            .expect("tag should match before the closing quote");
        assert_eq!(pos.start, 1);
        assert_eq!(pos.length, 5);
    }

    #[test]
    fn test_tag_in_quotes_resolves_without_space() {
        let tag_list = tag_list_with(&[("tag1", "hello")]);
        let content = "\"@tag1\"";
        let mut resolved = String::new();
        for item in content.to_special_char(|s| tag_list.tag_filter(s)) {
            match item {
                SpecialCharItem::Plain(s) => resolved.push_str(&s),
                SpecialCharItem::Special(s, _) => resolved.push_str(&tag_list.get_tagged_key(&s)),
            }
        }
        assert_eq!(resolved, "\"hello\"");
    }

    // Autocomplete pop-up navigation (issue #177).

    fn tag_list_showing(tags: &[(&str, &str)], pattern: &str) -> TagList {
        let mut tag_list = tag_list_with(tags);
        tag_list.update_pattern(pattern, pattern.chars().count());
        tag_list.update_autocomplete_list();
        tag_list
    }

    #[test]
    fn test_selection_starts_at_first_entry() {
        let tag_list = tag_list_showing(&[("alpha", "1"), ("beta", "2"), ("gamma", "3")], "@");
        assert_eq!(tag_list.selected(), 0);
        assert_eq!(
            tag_list
                .get_selected_autocomplete()
                .as_deref()
                .map(String::as_str),
            Some("alpha")
        );
    }

    #[test]
    fn test_select_next_and_prev_walk_the_list() {
        // Sorted case-insensitively -> [alpha, beta, gamma].
        let mut tag_list = tag_list_showing(&[("beta", "2"), ("alpha", "1"), ("gamma", "3")], "@");

        tag_list.select_next();
        assert_eq!(tag_list.selected(), 1);
        assert_eq!(
            tag_list
                .get_selected_autocomplete()
                .as_deref()
                .map(String::as_str),
            Some("beta")
        );

        tag_list.select_prev();
        assert_eq!(tag_list.selected(), 0);
    }

    #[test]
    fn test_selection_clamps_at_both_ends() {
        let mut tag_list = tag_list_showing(&[("alpha", "1"), ("beta", "2")], "@");

        // Cannot step above the first entry.
        tag_list.select_prev();
        assert_eq!(tag_list.selected(), 0);

        // Cannot step past the last entry.
        tag_list.select_next();
        tag_list.select_next();
        tag_list.select_next();
        assert_eq!(tag_list.selected(), 1);
    }

    #[test]
    fn test_selection_resets_when_list_is_rebuilt() {
        let mut tag_list = tag_list_showing(&[("alpha", "1"), ("beta", "2")], "@");
        tag_list.select_next();
        assert_eq!(tag_list.selected(), 1);

        // A keystroke rebuilds the list and returns the highlight to the top.
        tag_list.update_autocomplete_list();
        assert_eq!(tag_list.selected(), 0);
    }

    #[test]
    fn test_clear_resets_selection_and_hides_popup() {
        let mut tag_list = tag_list_showing(&[("alpha", "1"), ("beta", "2")], "@");
        tag_list.select_next();
        assert!(tag_list.has_suggestions());

        tag_list.clear();
        assert_eq!(tag_list.selected(), 0);
        assert!(!tag_list.has_suggestions());
        assert!(tag_list.get_selected_autocomplete().is_none());
    }

    #[test]
    fn test_has_suggestions_requires_pattern_and_matches() {
        // No pattern typed yet -> no pop-up.
        let empty = tag_list_with(&[("alpha", "1")]);
        assert!(!empty.has_suggestions());

        // Pattern with no matching tag -> no pop-up.
        let no_match = tag_list_showing(&[("alpha", "1")], "@zzz");
        assert!(!no_match.has_suggestions());

        // Pattern with matches -> pop-up shows.
        let matching = tag_list_showing(&[("alpha", "1")], "@al");
        assert!(matching.has_suggestions());
    }
}
