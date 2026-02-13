use std::borrow::Cow;

#[derive(Debug, PartialEq, Eq)]
pub enum SpecialCharItem {
    Plain(String),
    Special(String, usize),
}

pub struct SpecialCharPosition {
    pub start: usize,
    pub length: usize,
}

pub struct SpecialChar<F>
where
    F: FnMut(&str) -> Option<SpecialCharPosition>,
{
    content: String,
    special: Option<(String, usize)>,
    column: usize,
    filter: F,
}

pub trait ToSpecialChar {
    fn to_special_char<F>(self, filter: F) -> SpecialChar<F>
    where
        F: FnMut(&str) -> Option<SpecialCharPosition>;
}

impl From<(usize, usize)> for SpecialCharPosition {
    fn from(value: (usize, usize)) -> Self {
        Self {
            start: value.0,
            length: value.1,
        }
    }
}

impl ToSpecialChar for String {
    fn to_special_char<F>(self, filter: F) -> SpecialChar<F>
    where
        F: FnMut(&str) -> Option<SpecialCharPosition>,
    {
        SpecialChar {
            content: self,
            special: None,
            column: 0,
            filter,
        }
    }
}

impl ToSpecialChar for &str {
    fn to_special_char<F>(self, filter: F) -> SpecialChar<F>
    where
        F: FnMut(&str) -> Option<SpecialCharPosition>,
    {
        SpecialChar {
            content: self.to_string(),
            special: None,
            column: 0,
            filter,
        }
    }
}

impl<'a> ToSpecialChar for Cow<'a, str> {
    fn to_special_char<F>(self, filter: F) -> SpecialChar<F>
    where
        F: FnMut(&str) -> Option<SpecialCharPosition>,
    {
        SpecialChar {
            content: self.to_string(),
            special: None,
            column: 0,
            filter,
        }
    }
}

impl<F> Iterator for SpecialChar<F>
where
    F: FnMut(&str) -> Option<SpecialCharPosition>,
{
    type Item = SpecialCharItem;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((special, column)) = self.special.take() {
            return Some(SpecialCharItem::Special(special, column));
        }

        if self.content.is_empty() {
            return None;
        }

        let Some(SpecialCharPosition { start, length }) = (self.filter)(&self.content) else {
            let plain = self.content.drain(..).collect();
            return Some(SpecialCharItem::Plain(plain));
        };

        let plain = drain_chars_prefix(&mut self.content, start);
        let special = drain_chars_prefix(&mut self.content, length);
        self.special = Some((special, self.column + start));
        self.column += start + length;

        if !plain.is_empty() {
            Some(SpecialCharItem::Plain(plain))
        } else {
            let (special, column) = self.special.take().unwrap();
            Some(SpecialCharItem::Special(special, column))
        }
    }
}

fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(s.len())
}

fn drain_chars_prefix(s: &mut String, char_count: usize) -> String {
    let byte_idx = char_to_byte_idx(s, char_count);
    s.drain(..byte_idx).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic() {
        let content = "Hello \n World\\x03!".to_string();
        let iter = content.to_special_char(|string| {
            if let Some(start) = string.find("\n") {
                Some((start, 1).into())
            } else {
                string.find("\\x03").map(|start| (start, 4).into())
            }
        });
        let result = iter.collect::<Vec<_>>();
        let expected = [
            SpecialCharItem::Plain("Hello ".to_string()),
            SpecialCharItem::Special("\n".to_string(), 6),
            SpecialCharItem::Plain(" World".to_string()),
            SpecialCharItem::Special("\\x03".to_string(), 13),
            SpecialCharItem::Plain("!".to_string()),
        ];
        for (a, b) in result.iter().zip(expected.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_replace_from_start() {
        let content = "\x1b[mHello".to_string();
        let iter =
            content.to_special_char(|string| string.find("\x1b[m").map(|start| (start, 3).into()));
        let result = iter.collect::<Vec<_>>();
        let expected = [
            SpecialCharItem::Special("\x1b[m".to_string(), 0),
            SpecialCharItem::Plain("Hello".to_string()),
        ];
        for (a, b) in result.iter().zip(expected.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_replace_from_end() {
        let content = "Hello \x1b[m".to_string();
        let iter =
            content.to_special_char(|string| string.find("\x1b[m").map(|start| (start, 3).into()));
        let result = iter.collect::<Vec<_>>();
        let expected = [
            SpecialCharItem::Plain("Hello ".to_string()),
            SpecialCharItem::Special("\x1b[m".to_string(), 6),
        ];
        for (a, b) in result.iter().zip(expected.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_replace_seq_eq() {
        let content = "Hello \x1b[m \x1b[m".to_string();
        let iter =
            content.to_special_char(|string| string.find("\x1b[m").map(|start| (start, 3).into()));
        let result = iter.collect::<Vec<_>>();
        let expected = [
            SpecialCharItem::Plain("Hello ".to_string()),
            SpecialCharItem::Special("\x1b[m".to_string(), 6),
            SpecialCharItem::Plain(" ".to_string()),
            SpecialCharItem::Special("\x1b[m".to_string(), 10),
        ];
        for (a, b) in result.iter().zip(expected.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_replace_seq_diff() {
        let content = "Hello \x1b[m\x1b[8D".to_string();
        let iter = content.to_special_char(|string| {
            if let Some(start) = string.find("\x1b[m") {
                Some((start, 3).into())
            } else {
                string.find("\x1b[8D").map(|start| (start, 4).into())
            }
        });
        let result = iter.collect::<Vec<_>>();
        let expected = [
            SpecialCharItem::Plain("Hello ".to_string()),
            SpecialCharItem::Special("\x1b[m".to_string(), 6),
            SpecialCharItem::Special("\x1b[8D".to_string(), 9),
        ];
        for (a, b) in result.iter().zip(expected.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_long_string() {
        let content = "uart:~$ \x1b[m\x1b[8D\x1b[Juart:~$ \x1b[m".to_string();
        let iter = content.to_special_char(|string| {
            let mut min_pos = usize::MAX;
            let mut result = None;
            let patterns = [("\x1b[m", 3), ("\x1b[8D", 4), ("\x1b[J", 3)];
            for (pattern, length) in patterns.iter() {
                if let Some(start) = string.find(pattern)
                    && start < min_pos
                {
                    min_pos = start;
                    result = Some((start, *length).into());
                }
            }

            result
        });
        let result = iter.collect::<Vec<_>>();
        let expected = [
            SpecialCharItem::Plain("uart:~$ ".to_string()),
            SpecialCharItem::Special("\x1b[m".to_string(), 8),
            SpecialCharItem::Special("\x1b[8D".to_string(), 11),
            SpecialCharItem::Special("\x1b[J".to_string(), 15),
            SpecialCharItem::Plain("uart:~$ ".to_string()),
            SpecialCharItem::Special("\x1b[m".to_string(), 26),
        ];
        for (a, b) in result.iter().zip(expected.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_ansi() {
        let content = "Lorem ipsum dolor sit \\x1b[35mamet, consectetur adipiscing elit, sed do eiusm\\x1b[0mod tem\\r\\n".to_string();
        let iter = content.to_special_char(|string| {
            let mut min_pos = usize::MAX;
            let mut result = None;
            let patterns = [("\\r", 2), ("\\n", 2), ("\\x1b[35m", 8), ("\\x1b[0m", 7)];
            for (pattern, length) in patterns.iter() {
                if let Some(start) = string.find(pattern)
                    && start < min_pos
                {
                    min_pos = start;
                    result = Some((start, *length).into());
                }
            }

            result
        });
        let result = iter.collect::<Vec<_>>();
        let expected = [
            SpecialCharItem::Plain("Lorem ipsum dolor sit ".to_string()),
            SpecialCharItem::Special("\\x1b[35m".to_string(), 22),
            SpecialCharItem::Plain("amet, consectetur adipiscing elit, sed do eiusm".to_string()),
            SpecialCharItem::Special("\\x1b[0m".to_string(), 77),
            SpecialCharItem::Plain("od tem".to_string()),
            SpecialCharItem::Special("\\r".to_string(), 90),
            SpecialCharItem::Special("\\n".to_string(), 92),
        ];
        for (a, b) in result.iter().zip(expected.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_floating_letter() {
        let content1 = "{ectetur adipiscingelnit, sed d\\r\\n".to_string();
        let content2 = "{ectetur adipiscingelntt, sed d\\r\\n".to_string();
        let content3 = "{ectetur adipiscing elit, sed d\\r\\n".to_string();
        let filter = |string: &str| {
            if let Some(start) = string.find("\\r") {
                Some((start, 2).into())
            } else {
                string.find("\\n").map(|start| (start, 2).into())
            }
        };

        let iter1 = content1.to_special_char(filter);
        let iter2 = content2.to_special_char(filter);
        let iter3 = content3.to_special_char(filter);

        let results = [
            iter1.collect::<Vec<_>>(),
            iter2.collect::<Vec<_>>(),
            iter3.collect::<Vec<_>>(),
        ];
        let expected = [
            vec![
                SpecialCharItem::Plain("{ectetur adipiscingelnit, sed d".to_string()),
                SpecialCharItem::Special("\\r".to_string(), 31),
                SpecialCharItem::Special("\\n".to_string(), 33),
            ],
            vec![
                SpecialCharItem::Plain("{ectetur adipiscingelntt, sed d".to_string()),
                SpecialCharItem::Special("\\r".to_string(), 31),
                SpecialCharItem::Special("\\n".to_string(), 33),
            ],
            vec![
                SpecialCharItem::Plain("{ectetur adipiscing elit, sed d".to_string()),
                SpecialCharItem::Special("\\r".to_string(), 31),
                SpecialCharItem::Special("\\n".to_string(), 33),
            ],
        ];
        for (result, expected) in results.iter().zip(expected.iter()) {
            for (a, b) in result.iter().zip(expected.iter()) {
                assert_eq!(a, b);
            }
        }
    }

    #[test]
    fn test_query() {
        let content =
            "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tem\\r\\n"
                .to_string();
        let iter = content.to_special_char(|string| {
            if let Some(start) = string.find("dolor") {
                Some((start, 5).into())
            } else {
                string.find("sed").map(|start| (start, 3).into())
            }
        });
        let result = iter.collect::<Vec<_>>();
        let expected = [
            SpecialCharItem::Plain("Lorem ipsum ".to_string()),
            SpecialCharItem::Special("dolor".to_string(), 12),
            SpecialCharItem::Plain(" sit amet, consectetur adipiscing elit, ".to_string()),
            SpecialCharItem::Special("sed".to_string(), 57),
            SpecialCharItem::Plain(" do eiusmod tem\\r\\n".to_string()),
        ];
        for (a, b) in result.iter().zip(expected.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn test_no_special() {
        let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tem"
            .to_string();
        let iter = content.clone().to_special_char(|_| None);
        let result = iter.collect::<Vec<_>>();
        let expected = [SpecialCharItem::Plain(content)];
        for (a, b) in result.iter().zip(expected.iter()) {
            assert_eq!(a, b);
        }
    }
}
