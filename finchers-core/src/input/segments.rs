use super::encoded_str::EncodedStr;
use std::ops::Range;

/// An iterator over the remaining path segments.
#[derive(Debug, Copy, Clone)]
pub struct Segments<'a> {
    path: &'a str,
    pos: usize,
    popped: usize,
}

impl<'a> From<&'a str> for Segments<'a> {
    fn from(path: &'a str) -> Self {
        debug_assert!(!path.is_empty());
        debug_assert_eq!(path.chars().next(), Some('/'));
        Segments {
            path,
            pos: 1,
            popped: 0,
        }
    }
}

impl<'a> Segments<'a> {
    /// Returns the remaining path in this segments
    #[inline]
    pub fn remaining_path(&self) -> &'a str {
        &self.path[self.pos..]
    }

    /// Returns the cursor position in the original path
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Returns the number of segments already popped
    #[inline]
    pub fn popped(&self) -> usize {
        self.popped
    }
}

impl<'a> Iterator for Segments<'a> {
    type Item = Segment<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos == self.path.len() {
            return None;
        }
        if let Some(offset) = self.path[self.pos..].find('/') {
            let segment = Segment {
                s: self.path,
                range: Range {
                    start: self.pos,
                    end: self.pos + offset,
                },
            };
            self.pos += offset + 1;
            self.popped += 1;
            Some(segment)
        } else {
            let segment = Segment {
                s: self.path,
                range: Range {
                    start: self.pos,
                    end: self.path.len(),
                },
            };
            self.pos = self.path.len();
            self.popped += 1;
            Some(segment)
        }
    }
}

/// A path segment in the HTTP requests.
#[derive(Debug, Clone)]
pub struct Segment<'a> {
    s: &'a str,
    range: Range<usize>,
}

impl<'a> Segment<'a> {
    /// Create a `Segment` from a pair of path string and the range of segment.
    pub fn new(s: &'a str, range: Range<usize>) -> Segment<'a> {
        Segment { s, range }
    }

    /// Return an `EncodedStr` from this segment.
    pub fn as_encoded_str(&self) -> &'a EncodedStr {
        unsafe { EncodedStr::new_unchecked(self.s[self.range.clone()].as_bytes()) }
    }

    /// Returns the range of this segment in the original path.
    #[inline]
    pub fn as_range(&self) -> Range<usize> {
        self.range.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segments() {
        let mut segments = Segments::from("/foo/bar.txt");
        assert_eq!(segments.remaining_path(), "foo/bar.txt");
        assert_eq!(
            segments.next().map(|s| s.as_encoded_str().as_bytes()),
            Some(&b"foo"[..])
        );
        assert_eq!(segments.remaining_path(), "bar.txt");
        assert_eq!(
            segments.next().map(|s| s.as_encoded_str().as_bytes()),
            Some(&b"bar.txt"[..])
        );
        assert_eq!(segments.remaining_path(), "");
        assert_eq!(segments.next().map(|s| s.as_encoded_str().as_bytes()), None);
        assert_eq!(segments.remaining_path(), "");
        assert_eq!(segments.next().map(|s| s.as_encoded_str().as_bytes()), None);
    }

    #[test]
    fn test_segments_from_root_path() {
        let mut segments = Segments::from("/");
        assert_eq!(segments.remaining_path(), "");
        assert_eq!(segments.next().map(|s| s.as_encoded_str().as_bytes()), None);
    }

}