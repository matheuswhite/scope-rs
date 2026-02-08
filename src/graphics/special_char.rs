use std::{borrow::Cow, str::Chars};

#[derive(Debug, PartialEq, Eq)]
pub enum SpecialCharItem {
    Plain(String),
    Special(String, usize),
}

pub struct SpecialChar<'a, F>
where
    F: FnMut(&str) -> Option<usize>,
{
    content: Chars<'a>,
    special: Option<(String, usize)>,
    column: usize,
    filter: F,
}

pub trait ToSpecialChar {
    fn to_special_char<F>(&self, filter: F) -> SpecialChar<'_, F>
    where
        F: FnMut(&str) -> Option<usize>;
}

impl ToSpecialChar for String {
    fn to_special_char<F>(&self, filter: F) -> SpecialChar<'_, F>
    where
        F: FnMut(&str) -> Option<usize>,
    {
        SpecialChar {
            content: self.chars(),
            special: None,
            column: 0,
            filter,
        }
    }
}

impl ToSpecialChar for &str {
    fn to_special_char<F>(&self, filter: F) -> SpecialChar<'_, F>
    where
        F: FnMut(&str) -> Option<usize>,
    {
        SpecialChar {
            content: self.chars(),
            special: None,
            column: 0,
            filter,
        }
    }
}

impl<'a> ToSpecialChar for Cow<'a, str> {
    fn to_special_char<F>(&self, filter: F) -> SpecialChar<'_, F>
    where
        F: FnMut(&str) -> Option<usize>,
    {
        SpecialChar {
            content: self.chars(),
            special: None,
            column: 0,
            filter,
        }
    }
}

impl<'a, F> Iterator for SpecialChar<'a, F>
where
    F: FnMut(&str) -> Option<usize>,
{
    type Item = SpecialCharItem;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((special, column)) = self.special.take() {
            return Some(SpecialCharItem::Special(special, column));
        }

        let mut buffer = String::new();

        for letter in &mut self.content {
            buffer.push(letter);

            let Some(length) = (self.filter)(&buffer) else {
                continue;
            };

            let pivot = buffer.len().saturating_sub(length);
            let plain = buffer[..pivot].to_string();
            self.special = Some((buffer[pivot..].to_string(), self.column + pivot));
            self.column += buffer.len();

            if !plain.is_empty() {
                return Some(SpecialCharItem::Plain(plain));
            } else {
                let (special, column) = self.special.take().unwrap();
                return Some(SpecialCharItem::Special(special, column));
            }
        }

        if !buffer.is_empty() {
            return Some(SpecialCharItem::Plain(buffer));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic() {
        let content = "Hello \n World\\x03!".to_string();
        let iter = content.to_special_char(|string| {
            if string.contains("\n") {
                Some(1)
            } else if string.contains("\\x03") {
                Some(4)
            } else {
                None
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
        let iter = content.to_special_char(|string| {
            if string.ends_with("\x1b[m") {
                Some(3)
            } else {
                None
            }
        });
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
        let iter = content.to_special_char(|string| {
            if string.ends_with("\x1b[m") {
                Some(3)
            } else {
                None
            }
        });
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
        let iter = content.to_special_char(|string| {
            if string.ends_with("\x1b[m") {
                Some(3)
            } else {
                None
            }
        });
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
            if string.ends_with("\x1b[m") {
                Some(3)
            } else if string.ends_with("\x1b[8D") {
                Some(4)
            } else {
                None
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
            if string.ends_with("\x1b[m") {
                Some(3)
            } else if string.ends_with("\x1b[8D") {
                Some(4)
            } else if string.ends_with("\x1b[J") {
                Some(3)
            } else {
                None
            }
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
            if string.contains("\\r") || string.contains("\\n") {
                Some(2)
            } else if string.contains("\\x1b[35m") {
                Some(8)
            } else if string.contains("\\x1b[0m") {
                Some(7)
            } else {
                None
            }
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
            if string.contains("\\r") || string.contains("\\n") {
                Some(2)
            } else {
                None
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
            if string.contains("dolor") {
                Some(5)
            } else if string.contains("sed") {
                Some(3)
            } else {
                None
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
        let iter = content.to_special_char(|_| None);
        let result = iter.collect::<Vec<_>>();
        let expected = [SpecialCharItem::Plain(content)];
        for (a, b) in result.iter().zip(expected.iter()) {
            assert_eq!(a, b);
        }
    }
}
