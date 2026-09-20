//! Bounded views over a larger file.
//!
//! Every block of an installer is a self-contained byte range appended to
//! the file. A [`Window`] presents one as a `Read + Seek` stream of its own,
//! which is what the payload decoder and the hashers read.

use std::io::{self, Read, Seek, SeekFrom};

/// A read-only view of `[start, start + len)` of an inner stream.
///
/// The window keeps its own position and seeks the inner stream to it
/// before every read: a `File` cloned with `try_clone` shares one file
/// pointer with its clones, so two windows over one file would otherwise
/// move each other's position.
pub struct Window<R> {
    inner: R,
    start: u64,
    len: u64,
    position: u64,
}

impl<R: Read + Seek> Window<R> {
    pub fn new(mut inner: R, start: u64, len: u64) -> io::Result<Self> {
        inner.seek(SeekFrom::Start(start))?;
        Ok(Self {
            inner,
            start,
            len,
            position: 0,
        })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<R: Read + Seek> Read for Window<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.len.saturating_sub(self.position);
        if remaining == 0 {
            return Ok(0);
        }
        let limit = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        self.inner
            .seek(SeekFrom::Start(self.start + self.position))?;
        let n = self.inner.read(&mut buf[..limit])?;
        self.position += n as u64;
        Ok(n)
    }
}

impl<R: Read + Seek> Seek for Window<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        };
        let target = target.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "seek before start of window")
        })?;
        self.inner.seek(SeekFrom::Start(self.start + target))?;
        self.position = target;
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn window_reads_only_its_range() {
        let data: Vec<u8> = (0..100).collect();
        let mut window = Window::new(Cursor::new(data), 10, 20).unwrap();
        let mut all = Vec::new();
        window.read_to_end(&mut all).unwrap();
        assert_eq!(all, (10..30).collect::<Vec<u8>>());
        window.seek(SeekFrom::End(-5)).unwrap();
        let mut tail = Vec::new();
        window.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, vec![25, 26, 27, 28, 29]);
        assert_eq!(window.seek(SeekFrom::Start(3)).unwrap(), 3);
        let mut one = [0u8; 1];
        window.read_exact(&mut one).unwrap();
        assert_eq!(one[0], 13);
    }
}
