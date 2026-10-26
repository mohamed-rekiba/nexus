//! libarchive, loaded at run time, for the archives the Rust readers don't read: RAR (3, 4 and
//! 5), tar.xz, tar.bz2, tar.zst, ISO, cab, and 7z archives with a codec `sevenz-rust2` lacks.
//! Its readers are its own code, under the BSD 2-clause licence. The package ships a copy
//! (`scripts/bundle-libarchive.sh`): `lib/` beside the binary, `Contents/Frameworks` in the
//! macOS bundle, the binary's own folder on Windows. Without it, the system's is tried.

#![allow(unsafe_code)] // libarchive's C API, loaded at run time: each call follows archive.h and archive_entry.h.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

type Handle = *mut c_void;
type Unary = unsafe extern "C" fn(Handle) -> c_int;
type Text = unsafe extern "C" fn(Handle) -> *const c_char;
type OpenCallback = unsafe extern "C" fn(Handle, *mut c_void) -> c_int;
type ReadCallback = unsafe extern "C" fn(Handle, *mut c_void, *mut *const c_void) -> isize;
type CloseCallback = unsafe extern "C" fn(Handle, *mut c_void) -> c_int;
/// libarchive's `mode_t`: `unsigned short` on Windows.
#[cfg(unix)]
type Mode = libc::mode_t;
#[cfg(not(unix))]
type Mode = u16;

const ARCHIVE_OK: c_int = 0;
const ARCHIVE_EOF: c_int = 1;
/// At or above this a call worked, maybe with a warning (`ARCHIVE_WARN`).
const ARCHIVE_WARN: c_int = -20;

// `archive_entry_filetype`: the same values on every system.
const AE_IFMT: u32 = 0o170000;
const AE_IFREG: u32 = 0o100000;
const AE_IFDIR: u32 = 0o040000;
const AE_IFLNK: u32 = 0o120000;

/// Where libarchive may be, in order: the package's copy, then the system's.
fn candidates() -> Vec<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_default();
    let (shipped, system): (&[&str], &[&str]) = if cfg!(target_os = "macos") {
        (
            &["lib/libarchive.13.dylib", "../Frameworks/libarchive.13.dylib"],
            &[
                // Homebrew's keg-only libarchive isn't on the default search path.
                "/opt/homebrew/opt/libarchive/lib/libarchive.13.dylib",
                "/usr/local/opt/libarchive/lib/libarchive.13.dylib",
                "libarchive.13.dylib",
                // The system's own, in the shared cache: only loading it tells whether it is there.
                "/usr/lib/libarchive.2.dylib",
            ],
        )
    } else if cfg!(windows) {
        (&["libarchive-13.dll", "archive.dll"], &["libarchive-13.dll", "archive.dll"])
    } else {
        // lib/ beside the binary in the tarball; usr/lib beside usr/bin in the AppImage.
        (&["lib/libarchive.so.13", "../lib/libarchive.so.13"], &["libarchive.so.13", "libarchive.so"])
    };
    shipped.iter().map(|p| exe_dir.join(p)).chain(system.iter().map(PathBuf::from)).collect()
}

/// How to get libarchive on this system, for the error when it is missing.
pub fn install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "`brew install libarchive`"
    } else if cfg!(windows) {
        "a Nexus package, which ships it"
    } else {
        "libarchive13 on Debian and Ubuntu, libarchive on Fedora"
    }
}

pub struct Api {
    _lib: libloading::Library,
    new: unsafe extern "C" fn() -> Handle,
    formats: Unary,
    filters: Unary,
    #[cfg(not(windows))]
    open: unsafe extern "C" fn(Handle, *const *const c_char, usize) -> c_int,
    #[cfg(windows)]
    open: unsafe extern "C" fn(Handle, *const *const u16, usize) -> c_int,
    open_stream:
        unsafe extern "C" fn(Handle, *mut c_void, Option<OpenCallback>, ReadCallback, Option<CloseCallback>) -> c_int,
    next: unsafe extern "C" fn(Handle, *mut Handle) -> c_int,
    data: unsafe extern "C" fn(Handle, *mut c_void, usize) -> isize,
    skip: Unary,
    free: Unary,
    error: Text,
    pathname: Text,
    symlink: Text,
    hardlink: Text,
    size: unsafe extern "C" fn(Handle) -> i64,
    size_set: Unary,
    filetype: unsafe extern "C" fn(Handle) -> Mode,
    encrypted: Unary,
}

/// libarchive with RAR 5 support, loaded once; `None` when there is none.
pub fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| candidates().iter().find_map(|path| Api::load(path))).as_ref()
}

impl Api {
    fn load(path: &Path) -> Option<Self> {
        // SAFETY: loading a library runs its initialisers; libarchive's have no side effects.
        let lib = unsafe { libloading::Library::new(path) }.ok()?;
        // SAFETY: each type follows libarchive's public C API, and `lib` outlives every
        // pointer taken from it (it moves into the same struct). The `_utf8` names give
        // paths as UTF-8 on every system.
        let (api, rar5) = unsafe {
            macro_rules! sym {
                ($name:literal) => {
                    *lib.get(concat!($name, "\0").as_bytes()).ok()?
                };
            }
            let rar5: Unary = sym!("archive_read_support_format_rar5");
            let api = Self {
                new: sym!("archive_read_new"),
                formats: sym!("archive_read_support_format_all"),
                filters: sym!("archive_read_support_filter_all"),
                #[cfg(not(windows))]
                open: sym!("archive_read_open_filenames"),
                #[cfg(windows)]
                open: sym!("archive_read_open_filenames_w"),
                open_stream: sym!("archive_read_open"),
                next: sym!("archive_read_next_header"),
                data: sym!("archive_read_data"),
                skip: sym!("archive_read_data_skip"),
                free: sym!("archive_read_free"),
                error: sym!("archive_error_string"),
                pathname: sym!("archive_entry_pathname_utf8"),
                symlink: sym!("archive_entry_symlink_utf8"),
                hardlink: sym!("archive_entry_hardlink_utf8"),
                size: sym!("archive_entry_size"),
                size_set: sym!("archive_entry_size_is_set"),
                filetype: sym!("archive_entry_filetype"),
                encrypted: sym!("archive_entry_is_encrypted"),
                _lib: lib,
            };
            (api, rar5)
        };
        // A build without RAR 5 answers the support call with an error.
        // SAFETY: a new handle, freed at once.
        let ok = unsafe {
            let handle = (api.new)();
            if handle.is_null() {
                return None;
            }
            let ok = rar5(handle) == ARCHIVE_OK;
            (api.free)(handle);
            ok
        };
        ok.then_some(api)
    }
}

/// What an entry is.
#[derive(Debug, PartialEq)]
pub enum Kind {
    File,
    Folder,
    /// A link to its target, from the link's folder.
    Link(PathBuf),
    /// Another name for an entry, from the archive's top.
    HardLink(PathBuf),
    Other,
}

#[derive(Debug)]
pub struct Header {
    pub path: String,
    pub size: u64,
    pub kind: Kind,
    pub encrypted: bool,
}

/// The archive's parts' names as libarchive takes them: C strings, wide on Windows.
#[cfg(not(windows))]
type Names = Vec<std::ffi::CString>;
#[cfg(windows)]
type Names = Vec<Vec<u16>>;

#[cfg(not(windows))]
fn names(parts: &[PathBuf]) -> Result<Names, String> {
    use std::os::unix::ffi::OsStrExt;
    parts
        .iter()
        .map(|p| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|_| format!("{} has a nul", p.display())))
        .collect()
}

#[cfg(windows)]
fn names(parts: &[PathBuf]) -> Result<Names, String> {
    use std::os::windows::ffi::OsStrExt;
    Ok(parts.iter().map(|p| p.as_os_str().encode_wide().chain(Some(0)).collect()).collect())
}

/// One archive, read in order: a header, then its data or a skip.
pub struct Reader {
    api: &'static Api,
    handle: Handle,
    _names: Names,
    /// What [`Reader::open_at`] reads from; freed after the handle.
    _stream: Option<Box<Stream>>,
}

/// The rest of a file from a place in it, as libarchive reads a stream: forward only.
struct Stream {
    file: std::io::Take<std::fs::File>,
    buf: Vec<u8>,
}

/// libarchive's read callback for a [`Stream`]: the next piece, 0 at its end, -1 on an error.
unsafe extern "C" fn read_stream(_: Handle, data: *mut c_void, buffer: *mut *const c_void) -> isize {
    use std::io::Read;
    // SAFETY: `data` is the `Stream` that `open_at` gave libarchive, alive as long as the handle.
    let stream = unsafe { &mut *data.cast::<Stream>() };
    match stream.file.read(&mut stream.buf) {
        Ok(n) => {
            // SAFETY: libarchive gives a writable pointer; the piece lives until the next call.
            unsafe { *buffer = stream.buf.as_ptr().cast() };
            n as isize
        }
        Err(_) => -1,
    }
}

impl Reader {
    /// Opens the archive cut into `parts`, in order: libarchive reads them as one stream.
    pub fn open(api: &'static Api, parts: &[PathBuf]) -> Result<Self, String> {
        let names = names(parts)?;
        let mut pointers: Vec<_> = names.iter().map(|n| n.as_ptr()).collect();
        pointers.push(std::ptr::null());
        // SAFETY: a new handle; `Drop` frees it.
        let handle = unsafe { (api.new)() };
        if handle.is_null() {
            return Err("libarchive couldn't start".into());
        }
        let reader = Self { api, handle, _names: names, _stream: None };
        // SAFETY: `handle` is open, and `pointers` ends with a null and lives past the call;
        // libarchive keeps the names, which `_names` keeps alive.
        unsafe {
            reader.status((api.formats)(handle))?;
            reader.status((api.filters)(handle))?;
            reader.status((api.open)(handle, pointers.as_ptr(), 64 * 1024))?;
        }
        Ok(reader)
    }

    /// Opens the archive in the file at `path` from byte `offset` to its end, as a stream: a
    /// script installer's archive after its script.
    pub fn open_at(api: &'static Api, path: &Path, offset: u64) -> Result<Self, String> {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        file.seek(SeekFrom::Start(offset)).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut stream = Box::new(Stream { file: file.take(u64::MAX), buf: vec![0; 64 * 1024] });
        let data: *mut Stream = &mut *stream;
        // SAFETY: a new handle; `Drop` frees it.
        let handle = unsafe { (api.new)() };
        if handle.is_null() {
            return Err("libarchive couldn't start".into());
        }
        let reader = Self { api, handle, _names: Vec::new(), _stream: Some(stream) };
        // SAFETY: `handle` is open; the stream's box doesn't move and outlives the handle.
        unsafe {
            reader.status((api.formats)(handle))?;
            reader.status((api.filters)(handle))?;
            reader.status((api.open_stream)(handle, data.cast(), None, read_stream, None))?;
        }
        Ok(reader)
    }

    fn status(&self, code: c_int) -> Result<(), String> {
        if code >= ARCHIVE_WARN {
            return Ok(());
        }
        // SAFETY: the handle is open; the string, when there is one, lives until the next call.
        let message = unsafe {
            let p = (self.api.error)(self.handle);
            if p.is_null() {
                "it isn't an archive libarchive reads, or a part is missing".into()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        let lower = message.to_ascii_lowercase();
        if lower.contains("encrypt") || lower.contains("password") || lower.contains("passphrase") {
            return Err(crate::archives::password());
        }
        Err(format!("the archive couldn't be read: {message}"))
    }

    /// The next entry, or `None` at the end.
    pub fn next(&mut self) -> Result<Option<Header>, String> {
        let mut entry: Handle = std::ptr::null_mut();
        // SAFETY: the handle is open and `entry` is a writable pointer.
        let code = unsafe { (self.api.next)(self.handle, &mut entry) };
        if code == ARCHIVE_EOF {
            return Ok(None);
        }
        self.status(code)?;
        if entry.is_null() {
            return Err("the archive has an entry with no header".into());
        }
        let text = |p: *const c_char| -> Option<String> {
            // SAFETY: libarchive's entry strings live until the next header.
            (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
        };
        // SAFETY: `entry` is the header libarchive just read.
        unsafe {
            let path = text((self.api.pathname)(entry)).ok_or("the archive has an entry with no name")?;
            // `Mode` is u16 on Windows and macOS and u32 on Linux, where the conversion does nothing.
            #[allow(clippy::useless_conversion)]
            let kind = u32::from((self.api.filetype)(entry)) & AE_IFMT;
            let kind = if let Some(target) = text((self.api.hardlink)(entry)) {
                Kind::HardLink(target.into())
            } else if let Some(target) = text((self.api.symlink)(entry)).filter(|_| kind == AE_IFLNK || kind == 0) {
                Kind::Link(target.into())
            } else if kind == AE_IFDIR {
                Kind::Folder
            } else if kind == AE_IFREG {
                Kind::File
            } else {
                Kind::Other
            };
            let size = if (self.api.size_set)(entry) != 0 { (self.api.size)(entry).max(0) as u64 } else { 0 };
            let encrypted = (self.api.encrypted)(entry) > 0;
            Ok(Some(Header { path, size, kind, encrypted }))
        }
    }

    /// Skips the data of the entry `next` returned.
    pub fn skip(&mut self) -> Result<(), String> {
        // SAFETY: the handle is open, after a header.
        let code = unsafe { (self.api.skip)(self.handle) };
        self.status(code)
    }
}

impl std::io::Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // SAFETY: the handle is open, after a header, and `buf` is writable for its length.
        let n = unsafe { (self.api.data)(self.handle, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let message = self.status(n as c_int).err().unwrap_or_else(|| "the data couldn't be read".into());
            return Err(std::io::Error::other(message));
        }
        Ok(n as usize)
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // SAFETY: the handle is open, and this is its one free.
        unsafe {
            (self.api.free)(self.handle);
        }
    }
}
