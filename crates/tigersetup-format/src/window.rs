//! Bounded views over a larger file.
//!
//! The payload is a standalone ZIP archive appended verbatim to the installer,
//! so its internal offsets are relative to the block. A [`Window`] presents
//! the block as a self-contained `Read + Seek` stream; an [`OffsetWriter`]
//! lets the ZIP writer produce block-relative offsets while writing straight
//! into the output file.

use std::io::{self, Read, Seek, SeekFrom, Write};

/// A read-only view of `[start, start + len)` of an inner stream.
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

/// A writer whose reported positions start at zero at `base` of the inner
/// stream. Seeks are translated the same way, so a ZIP written through it is
/// a self-consistent archive once copied out as `[base, end)`.
pub struct OffsetWriter<W> {
    inner: W,
    base: u64,
    position: u64,
}

impl<W: Write + Seek> OffsetWriter<W> {
    pub fn new(mut inner: W) -> io::Result<Self> {
        let base = inner.stream_position()?;
        Ok(Self {
            inner,
            base,
            position: 0,
        })
    }

    pub fn into_inner(self) -> W {
        self.inner
    }

    pub fn base(&self) -> u64 {
        self.base
    }
}

impl<W: Write + Seek> Write for OffsetWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.position += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<W: Write + Seek> Seek for OffsetWriter<W> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let absolute = match pos {
            SeekFrom::Start(offset) => self.inner.seek(SeekFrom::Start(self.base + offset))?,
            SeekFrom::Current(delta) => self.inner.seek(SeekFrom::Current(delta))?,
            SeekFrom::End(delta) => self.inner.seek(SeekFrom::End(delta))?,
        };
        self.position = absolute.checked_sub(self.base).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start of the block",
            )
        })?;
        Ok(self.position)
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

    #[test]
    fn offset_writer_reports_block_relative_positions() {
        let mut cursor = Cursor::new(Vec::new());
        cursor.write_all(b"prefix").unwrap();
        let mut writer = OffsetWriter::new(cursor).unwrap();
        assert_eq!(writer.base(), 6);
        writer.write_all(b"hello").unwrap();
        assert_eq!(writer.stream_position().unwrap(), 5);
        writer.seek(SeekFrom::Start(1)).unwrap();
        writer.write_all(b"J").unwrap();
        assert_eq!(writer.into_inner().into_inner(), b"prefixhJllo");
    }
}
