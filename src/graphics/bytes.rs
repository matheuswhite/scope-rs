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
        vec: Some(vec.as_slice()),
        from,
    }
}

struct BytesMatchIndices<'a> {
    vec: Option<&'a [u8]>,
    from: &'a [u8],
}

impl<'a> Iterator for BytesMatchIndices<'a> {
    type Item = (usize, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let Some(vec) = self.vec.take() else {
            return None;
        };

        for (i, _byte) in vec.iter().enumerate() {
            let start_idx = i;
            let end_idx = i + self.from.len();

            if vec.get(start_idx..end_idx) == Some(self.from) {
                self.vec = vec.get(end_idx..);
                return Some((i, self.from));
            }
        }

        None
    }
}
