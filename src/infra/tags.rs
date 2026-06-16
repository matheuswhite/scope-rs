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

    pub fn get_first_autocomplete_list(&self) -> Option<Arc<String>> {
        self.autocomplete_list.first().cloned()
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
    }

    pub fn clear(&mut self) {
        self.pattern = Arc::new(String::new());
        self.autocomplete_list.clear();
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
                .take_while(|c| !c.is_whitespace() && *c != '@')
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
        let pos = tag_list.tag_filter("@tag1@tag2").expect("first tag should match");
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
}
