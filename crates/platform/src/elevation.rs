//! Whether FilmCraft runs with administrator rights (Windows). Windows' User Interface Privilege
//! Isolation then drops every file drag-and-drop from Explorer, which runs without them, so files
//! can't be dropped on the Project panel or the Timeline. The app says so at startup.
//!
//! FFI module (docs/adr/0001-platform-ffi.md): every `unsafe` block has a `// SAFETY:` comment and
//! the public item is safe.

/// True when this process runs elevated ("Run as administrator"). Always false elsewhere.
pub fn running_elevated() -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        let mut token = HANDLE::default();
        // SAFETY: GetCurrentProcess returns a pseudo handle; `token` is a valid out pointer.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        // SAFETY: the buffer is a TOKEN_ELEVATION of the size passed; `token` was opened above.
        let ok = unsafe {
            GetTokenInformation(token, TokenElevation, Some(std::ptr::from_mut(&mut elevation).cast()), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len)
        }
        .is_ok();
        // SAFETY: `token` is a handle this function opened and owns.
        let _ = unsafe { CloseHandle(token) };
        ok && elevation.TokenIsElevated != 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}
