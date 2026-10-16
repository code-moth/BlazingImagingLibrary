// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Reading a Source's bytes at stated places, from a file or from memory, on any thread.

use std::io;

/// Bytes that can be read at any place, from any thread, without a shared position.
pub trait ReadAt: Send + Sync {
    /// The number of bytes.
    fn size(&self) -> io::Result<u64>;
    /// Fills `buf` with the bytes from `offset` on, or fails.
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()>;
}

fn past_end() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "read past the end")
}

fn from_slice(bytes: &[u8], buf: &mut [u8], offset: u64) -> io::Result<()> {
    let start = usize::try_from(offset).map_err(|_| past_end())?;
    let end = start.checked_add(buf.len()).ok_or_else(past_end)?;
    buf.copy_from_slice(bytes.get(start..end).ok_or_else(past_end)?);
    Ok(())
}

impl<T: ReadAt + ?Sized> ReadAt for std::sync::Arc<T> {
    fn size(&self) -> io::Result<u64> {
        (**self).size()
    }
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        (**self).read_exact_at(buf, offset)
    }
}

impl ReadAt for Vec<u8> {
    fn size(&self) -> io::Result<u64> {
        Ok(self.len() as u64)
    }
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        from_slice(self, buf, offset)
    }
}

impl ReadAt for &[u8] {
    fn size(&self) -> io::Result<u64> {
        Ok(self.len() as u64)
    }
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        from_slice(self, buf, offset)
    }
}

impl ReadAt for std::fs::File {
    fn size(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    #[cfg(unix)]
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(self, buf, offset)
    }

    #[cfg(windows)]
    fn read_exact_at(&self, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
        // seek_read moves the file's position, which nothing here relies on.
        while !buf.is_empty() {
            match std::os::windows::fs::FileExt::seek_read(self, buf, offset) {
                Ok(0) => return Err(past_end()),
                Ok(n) => {
                    buf = &mut buf[n..];
                    offset += n as u64;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    fn read_exact_at(&self, _buf: &mut [u8], _offset: u64) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "reading at a place is not available on this system",
        ))
    }
}
