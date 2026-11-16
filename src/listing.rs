//! One-directory listing with sizes, platform-specific so that allocated size comes
//! from the listing itself rather than an extra system call per file.

use std::io;
use std::path::{Path, PathBuf};

pub enum Kind {
    File,
    Dir(PathBuf),
}

pub struct Item<'a> {
    pub name: &'a str,
    pub kind: Kind,
    /// Logical length in bytes.
    pub size: u64,
    /// Bytes actually allocated on disk: cluster-rounded, smaller for compressed
    /// and sparse files, 0 for cloud-only placeholders (OneDrive "online-only").
    pub allocated: u64,
    /// Unix seconds; `crate::tree::unix_secs(None)` if unknown.
    pub mtime: i64,
}

/// Lists `dir`, calling `f` for each file and subdirectory. Symlinks and junctions are
/// skipped. Returns `Ok(true)` if some entries could not be read.
///
/// `same_fs` is the device to stay on (Unix only; ignored on Windows).
#[cfg(windows)]
pub fn list_dir(dir: &Path, _same_fs: Option<u64>, mut f: impl FnMut(Item)) -> io::Result<bool> {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FULL_DIR_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FileFullDirectoryInfo, FileFullDirectoryRestartInfo, GetFileInformationByHandleEx, OPEN_EXISTING,
    };

    /// Reparse tags with this bit redirect to another name: symlinks, junctions, mount points.
    const NAME_SURROGATE: u32 = 0x2000_0000;
    /// 100 ns intervals between 1601-01-01 and 1970-01-01.
    const EPOCH_DIFF_SECS: i64 = 11_644_473_600;

    struct Handle(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful CreateFileW and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    let wide: Vec<u16> = verbatim(dir).as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call; null security attributes are allowed.
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let handle = Handle(raw);

    // u64 storage keeps the buffer 8-byte aligned, as FILE_FULL_DIR_INFO requires.
    let mut buf = vec![0u64; 64 * 1024 / 8];
    let mut name = String::new();
    let mut class = FileFullDirectoryRestartInfo;
    loop {
        // SAFETY: the buffer is valid, writable and aligned for the given byte size.
        let ok = unsafe {
            GetFileInformationByHandleEx(handle.0, class, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32)
        };
        if ok == 0 {
            // SAFETY: plain thread-local error query.
            return match unsafe { GetLastError() } {
                ERROR_NO_MORE_FILES => Ok(false),
                code => Err(io::Error::from_raw_os_error(code as i32)),
            };
        }
        class = FileFullDirectoryInfo;

        let base = buf.as_ptr().cast::<u8>();
        let mut offset = 0usize;
        loop {
            // SAFETY: the kernel filled the buffer with a chain of FILE_FULL_DIR_INFO records,
            // each 8-byte aligned and linked by NextEntryOffset; the name follows the header.
            let (info, name_units) = unsafe {
                let info = &*base.add(offset).cast::<FILE_FULL_DIR_INFO>();
                let units = std::slice::from_raw_parts(
                    std::ptr::addr_of!(info.FileName).cast::<u16>(),
                    info.FileNameLength as usize / 2,
                );
                (info, units)
            };

            let is_dot = matches!(name_units, [0x2E] | [0x2E, 0x2E]);
            let reparse = info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
            // For reparse points, EaSize holds the reparse tag.
            let link = reparse && info.EaSize & NAME_SURROGATE != 0;
            if !is_dot && !link {
                name.clear();
                name.extend(char::decode_utf16(name_units.iter().copied()).map(|c| c.unwrap_or('\u{FFFD}')));
                let is_dir = info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
                let kind = if is_dir { Kind::Dir(dir.join(OsString::from_wide(name_units))) } else { Kind::File };
                f(Item {
                    name: &name,
                    kind,
                    size: if is_dir { 0 } else { info.EndOfFile.max(0) as u64 },
                    allocated: if is_dir { 0 } else { info.AllocationSize.max(0) as u64 },
                    mtime: info.LastWriteTime / 10_000_000 - EPOCH_DIFF_SECS,
                });
            }

            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
        }
    }
}

/// `\\?\` form of an absolute path, so paths beyond MAX_PATH work.
#[cfg(windows)]
fn verbatim(path: &Path) -> PathBuf {
    use std::path::{Component, Prefix};
    match path.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(_) => {
                let mut s = std::ffi::OsString::from(r"\\?\");
                s.push(path.as_os_str());
                PathBuf::from(s)
            }
            Prefix::UNC(..) => {
                let mut s = std::ffi::OsString::from(r"\\?\UNC");
                s.push(&path.as_os_str().to_string_lossy()[1..]);
                PathBuf::from(s)
            }
            _ => path.to_path_buf(), // already verbatim or a device path
        },
        _ => path.to_path_buf(),
    }
}

#[cfg(not(windows))]
pub fn list_dir(dir: &Path, same_fs: Option<u64>, mut f: impl FnMut(Item)) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let mut partial = false;
    for entry in std::fs::read_dir(dir)? {
        let Ok((entry, file_type, meta)) = entry.and_then(|e| Ok((e.file_type()?, e.metadata()?)).map(|(t, m)| (e, t, m)))
        else {
            partial = true;
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let mtime = crate::tree::unix_secs(meta.modified().ok());
        if file_type.is_dir() {
            if same_fs.is_some_and(|dev| dev != meta.dev()) {
                continue;
            }
            f(Item { name: &name, kind: Kind::Dir(entry.path()), size: 0, allocated: 0, mtime });
        } else {
            // st_blocks is always in 512-byte units.
            f(Item { name: &name, kind: Kind::File, size: meta.len(), allocated: meta.blocks() * 512, mtime });
        }
    }
    Ok(partial)
}
