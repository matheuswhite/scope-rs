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
        let tag_file_content = std::fs::read_to_string(file_path)
            .map_err(|_| format!("Cannot read tag file at {}", file_path.display()))?;
        serde_yaml::from_str(&tag_file_content)
            .map_err(|_| format!("Failed to parse tag file at {}", file_path.display()))
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

        let at_pos = text[..cursor].rfind('@').unwrap_or(0);

        if text[at_pos..cursor].chars().any(|c| c.is_whitespace()) {
            self.pattern = Arc::new(String::new());
            return;
        }

        self.pattern = Arc::new(text[at_pos..cursor].to_string());
    }

    pub fn update_autocomplete_list(&mut self) {
        self.autocomplete_list = self
            .tags
            .keys()
            .filter(|&k| match self.pattern.len() {
                0 => false,
                1 if self.pattern.as_str() == "@" => true,
                1 if self.pattern.as_str() != "@" => false,
                _ => {
                    k != &self.pattern[1..]
                        && &self.pattern[0..1] == "@"
                        && k.starts_with(&self.pattern[1..])
                }
            })
            .map(|k| Arc::new(k.clone()))
            .collect();
        self.autocomplete_list.sort_by_key(|a| a.to_lowercase());
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
        let at_pos = string.find('@')?;

        let tag_name = string[at_pos..]
            .strip_prefix('@')?
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect::<String>();
        if self.tags.contains_key(&tag_name) {
            Some((at_pos, tag_name.len() + 1).into())
        } else {
            None
        }
    }

    pub fn autocomplete_list(&self) -> Vec<Arc<String>> {
        self.autocomplete_list.clone()
    }

    pub fn pattern(&self) -> Arc<String> {
        self.pattern.clone()
    }
}
