pub trait SliceExt<T> {
    fn contains(&self, pat: &[T]) -> bool;
    fn replace(&self, from: &[T], to: &[T]) -> Self;
}

impl SliceExt<u8> for Vec<u8> {
    fn contains(&self, pat: &[u8]) -> bool {
        self.iter()
            .enumerate()
            .any(|(i, byte)| pat.get(0) == Some(byte) && Some(pat) == self.get(i..(i + pat.len())))
    }

    fn replace(&self, from: &[u8], to: &[u8]) -> Self {
        let mut result = vec![];
        let mut last_end = 0;
        for (start, part) in match_indices(self, from) {
            result.extend_from_slice(unsafe { self.get_unchecked(last_end..start) });
            result.extend_from_slice(to);
            last_end = start + part.len();
        }
        result.extend_from_slice(unsafe { self.get_unchecked(last_end..self.len()) });
        result
    }
}

fn match_indices<'a>(vec: &'a Vec<u8>, from: &'a [u8]) -> BytesMatchIndices<'a> {
    BytesMatchIndices {
        vec: vec.as_slice(),
        offset: 0,
        from,
    }
}

struct BytesMatchIndices<'a> {
    vec: &'a [u8],
    offset: usize,
    from: &'a [u8],
}

impl<'a> Iterator for BytesMatchIndices<'a> {
    type Item = (usize, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.vec.len() {
            return None;
        }

        for (i, _byte) in self.vec.iter().enumerate() {
            let start_idx = i + self.offset;
            let end_idx = i + self.offset + self.from.len();

            if self.vec.get(start_idx..end_idx) == Some(self.from) {
                self.offset = end_idx;
                return Some((start_idx, self.from));
            }
        }

        None
    }
}

#[cfg(test)]
mod test {
    use super::SliceExt;

    #[test]
    fn test_replace_from_start() {
        let expected = b"Hello".to_vec();
        let res = b"\x1b[mHello".to_vec().replace(b"\x1b[m", b"");

        assert_eq!(expected, res);
    }

    #[test]
    fn test_replace_from_end() {
        let expected = b"Hello ".to_vec();
        let res = b"Hello \x1b[m".to_vec().replace(b"\x1b[m", b"");

        assert_eq!(expected, res);
    }

    #[test]
    fn test_replace_seq_eq() {
        let expected = b"Hello ".to_vec();
        let res = b"Hello\x1b[m \x1b[m".to_vec().replace(b"\x1b[m", b"");

        assert_eq!(expected, res);
    }

    #[test]
    fn test_replace_seq_diff() {
        let expected = b"Hello\x1b[8D".to_vec();
        let res = b"Hello\x1b[m\x1b[8D".to_vec().replace(b"\x1b[m", b"");

        assert_eq!(expected, res);
    }

    #[test]
    fn test_long_string() {
        let expected = b"uart:~$ uart:~$ ".to_vec();
        let res = b"uart:~$ \x1b[m\x1b[8D\x1b[Juart:~$ \x1b[m"
            .to_vec()
            .replace(b"\x1b[m", b"")
            .replace(b"\x1b[8D", b"")
            .replace(b"\x1b[J", b"");

        assert_eq!(expected, res);
    }

    #[test]
    fn test_long_string_out_of_order() {
        let expected = b"uart:~$ uart:~$ ".to_vec();
        let res = b"uart:~$ \x1b[m\x1b[8D\x1b[Juart:~$ \x1b[m"
            .to_vec()
            .replace(b"\x1b[m", b"")
            .replace(b"\x1b[J", b"")
            .replace(b"\x1b[8D", b"");
        let res2 = b"uart:~$ \x1b[m\x1b[8D\x1b[Juart:~$ \x1b[m"
            .to_vec()
            .replace(b"\x1b[J", b"")
            .replace(b"\x1b[8D", b"")
            .replace(b"\x1b[m", b"");
        let res3 = b"uart:~$ \x1b[m\x1b[8D\x1b[Juart:~$ \x1b[m"
            .to_vec()
            .replace(b"\x1b[8D", b"")
            .replace(b"\x1b[m", b"")
            .replace(b"\x1b[J", b"");

        assert_eq!(expected, res);
        assert_eq!(expected, res2);
        assert_eq!(expected, res3);
    }
}
