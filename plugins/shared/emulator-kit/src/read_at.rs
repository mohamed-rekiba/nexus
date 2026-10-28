//! What the package readers read from: a file on disk, or a file in a host folder that a
//! plugin reaches through [`nexus_api::Folders`].

use std::io;
use std::sync::Arc;

use nexus_api::{Folders, HostPath, MAX_FOLDER_READ};

/// Reads at an offset, like `pread`. Never moves a cursor, so a reader can share it.
pub trait ReadAt {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()>;
    fn len(&self) -> io::Result<u64>;
    fn is_empty(&self) -> io::Result<bool> {
        Ok(self.len()? == 0)
    }
}

impl ReadAt for std::fs::File {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::FileExt::read_exact_at(self, buf, offset)
        }
        #[cfg(windows)]
        {
            let mut done = 0;
            while done < buf.len() {
                let n = std::os::windows::fs::FileExt::seek_read(self, &mut buf[done..], offset + done as u64)?;
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                done += n;
            }
            Ok(())
        }
    }

    fn len(&self) -> io::Result<u64> {
        Ok(self.metadata()?.len())
    }
}

impl ReadAt for [u8] {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| io::Error::from(io::ErrorKind::UnexpectedEof))?;
        let src = start.checked_add(buf.len()).and_then(|end| self.get(start..end));
        buf.copy_from_slice(src.ok_or(io::ErrorKind::UnexpectedEof)?);
        Ok(())
    }

    fn len(&self) -> io::Result<u64> {
        Ok(<[u8]>::len(self) as u64)
    }
}

impl ReadAt for Vec<u8> {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        self.as_slice().read_exact_at(buf, offset)
    }

    fn len(&self) -> io::Result<u64> {
        Ok(self.as_slice().len() as u64)
    }
}

/// A file in a host folder, read through the plugin's folders facade. For blocking code: it
/// waits on the runtime, so it must run on a blocking thread (`spawn_blocking`), never in an
/// async task.
pub struct HostFile {
    pub folders: Arc<dyn Folders>,
    pub path: HostPath,
    pub size: u64,
    pub runtime: tokio::runtime::Handle,
}

impl HostFile {
    /// `size` bytes at `path`, read from blocking code later. Call it from async code.
    pub fn new(folders: Arc<dyn Folders>, path: HostPath, size: u64) -> Self {
        Self { folders, path, size, runtime: tokio::runtime::Handle::current() }
    }
}

impl ReadAt for HostFile {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let mut done = 0;
        while done < buf.len() {
            let want = (buf.len() - done).min(MAX_FOLDER_READ as usize) as u64;
            let piece = self
                .runtime
                .block_on(self.folders.read_range(&self.path, offset + done as u64, want))
                .map_err(|e| io::Error::other(e.to_string()))?;
            if piece.is_empty() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            buf[done..done + piece.len()].copy_from_slice(&piece);
            done += piece.len();
        }
        Ok(())
    }

    fn len(&self) -> io::Result<u64> {
        Ok(self.size)
    }
}
