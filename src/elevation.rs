//! Administrator access on Windows: detect elevation, relaunch elevated, and enable
//! the backup privilege so an elevated scan can list folders whose ACLs exclude even
//! administrators (e.g. `System Volume Information`).

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LUID};
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, GetTokenInformation, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_BACKUP_NAME,
        SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    struct Token(HANDLE);

    impl Token {
        fn open(access: u32) -> Option<Self> {
            let mut handle: HANDLE = std::ptr::null_mut();
            // SAFETY: the current-process pseudo handle is always valid; `handle` is a valid out pointer.
            let ok = unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut handle) };
            (ok != 0).then_some(Self(handle))
        }
    }

    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful OpenProcessToken and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// True when not elevated, i.e. restarting as administrator would gain access.
    pub fn can_relaunch_elevated() -> bool {
        !is_elevated()
    }

    fn is_elevated() -> bool {
        let Some(token) = Token::open(TOKEN_QUERY) else { return false };
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        // SAFETY: the buffer is a TOKEN_ELEVATION of exactly the size passed.
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenElevation,
                (&raw mut elevation).cast(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
        };
        ok != 0 && elevation.TokenIsElevated != 0
    }

    /// Enables SeBackupPrivilege if the token holds it (elevated administrators do).
    /// Directory handles opened with FILE_FLAG_BACKUP_SEMANTICS then bypass read ACLs.
    pub fn enable_backup_privilege() {
        let Some(token) = Token::open(TOKEN_ADJUST_PRIVILEGES) else { return };
        let mut luid = LUID { LowPart: 0, HighPart: 0 };
        // SAFETY: SE_BACKUP_NAME is a static NUL-terminated wide string; `luid` is a valid out pointer.
        if unsafe { LookupPrivilegeValueW(std::ptr::null(), SE_BACKUP_NAME, &mut luid) } == 0 {
            return;
        }
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }],
        };
        // SAFETY: `privileges` is a valid TOKEN_PRIVILEGES; no previous state is requested.
        // Failure (privilege not held) leaves the token unchanged, which is fine.
        unsafe { AdjustTokenPrivileges(token.0, 0, &privileges, 0, std::ptr::null_mut(), std::ptr::null_mut()) };
    }

    /// Starts this executable again through the UAC prompt with `args`.
    /// Errors if the user declines the prompt.
    pub fn relaunch_elevated(args: &[&OsStr]) -> io::Result<()> {
        let exe = std::env::current_exe()?;
        let wide = |s: &OsStr| s.encode_wide().chain([0]).collect::<Vec<u16>>();
        let mut params = std::ffi::OsString::new();
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                params.push(" ");
            }
            params.push(quote(arg));
        }
        let (verb, file, params) = (wide(OsStr::new("runas")), wide(exe.as_os_str()), wide(&params));
        // SAFETY: all strings are NUL-terminated and outlive the call; null window and directory are allowed.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                params.as_ptr(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // Per the ShellExecute contract, values above 32 mean success.
        if result as usize > 32 { Ok(()) } else { Err(io::Error::last_os_error()) }
    }

    /// Quotes one argument for the MSVC command-line parser.
    fn quote(arg: &OsStr) -> std::ffi::OsString {
        let s = arg.to_string_lossy();
        let mut out = String::from("\"");
        let mut backslashes = 0;
        for c in s.chars() {
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                    out.push('"');
                    backslashes = 0;
                    continue;
                }
                _ => {}
            }
            if c != '\\' {
                out.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
        // Backslashes before the closing quote must be doubled.
        out.extend(std::iter::repeat_n('\\', backslashes * 2));
        out.push('"');
        out.into()
    }

    #[cfg(test)]
    mod tests {
        use super::quote;
        use std::ffi::OsStr;

        #[test]
        fn quotes_like_msvc_expects() {
            assert_eq!(quote(OsStr::new(r"C:\")), r#""C:\\""#);
            assert_eq!(quote(OsStr::new(r"C:\Program Files")), r#""C:\Program Files""#);
            assert_eq!(quote(OsStr::new(r#"a"b"#)), r#""a\"b""#);
        }
    }
}

#[cfg(windows)]
pub use imp::{can_relaunch_elevated, enable_backup_privilege, relaunch_elevated};

/// Relaunching elevated is only offered on Windows.
#[cfg(not(windows))]
pub fn can_relaunch_elevated() -> bool {
    false
}

#[cfg(not(windows))]
pub fn enable_backup_privilege() {}
