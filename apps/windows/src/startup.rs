//! Manual launches open the window; background launches reuse the same instance.

pub const WINDOW_CLASS: &str = "TTreePureRustWin32MainClass";

pub fn should_show(args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> bool {
    let mut hidden = false;
    for arg in args {
        if arg.as_ref() == "--show" {
            return true;
        }
        hidden |= arg.as_ref() == "--hidden";
    }
    !hidden
}

#[cfg(target_os = "windows")]
pub use instance::{acquire, report_error};

#[cfg(target_os = "windows")]
mod instance {
    use std::{ffi::c_void, io, ptr, thread, time::Duration};

    type Handle = *mut c_void;
    const ERROR_ALREADY_EXISTS: u32 = 183;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(attributes: *const c_void, owner: i32, name: *const u16) -> Handle;
        fn GetLastError() -> u32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> Handle;
        fn GetWindowThreadProcessId(hwnd: Handle, pid: *mut u32) -> u32;
        fn AllowSetForegroundWindow(pid: u32) -> i32;
        fn PostMessageW(hwnd: Handle, message: u32, wparam: usize, lparam: isize) -> i32;
        fn MessageBoxW(hwnd: Handle, text: *const u16, title: *const u16, flags: u32) -> i32;
    }

    pub struct Instance(Handle);

    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    fn forward(show: bool) -> io::Result<bool> {
        let class = wide(super::WINDOW_CLASS);
        unsafe {
            let hwnd = FindWindowW(class.as_ptr(), ptr::null());
            if hwnd.is_null() {
                return Ok(false);
            }
            if show {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, &mut pid);
                AllowSetForegroundWindow(pid);
                // Reuse the tray's translation action, including for older running builds.
                // Posting waits for startup to finish before the main message loop handles it.
                if PostMessageW(hwnd, 0x0111, 901, 0) == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(true)
        }
    }

    pub fn acquire(show: bool) -> io::Result<Option<Instance>> {
        // Older builds have no mutex, but can still receive the tray action.
        if forward(show)? {
            return Ok(None);
        }
        let name = wide("Local\\com.samwstu.ttree.windows");
        let (handle, error) = unsafe {
            let handle = CreateMutexW(ptr::null(), 0, name.as_ptr());
            (handle, GetLastError())
        };
        if handle.is_null() {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        let instance = Instance(handle);
        if error != ERROR_ALREADY_EXISTS {
            return Ok(Some(instance));
        }
        if !show {
            return Ok(None);
        }
        // A simultaneous launch may still be loading settings and creating its window.
        for _ in 0..100 {
            if forward(show)? {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "已有 TTREE 实例尚未响应，请稍后重试。",
        ))
    }

    pub fn report_error(error: &io::Error) {
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                wide(&format!("启动 TTREE 失败：{error}")).as_ptr(),
                wide("TTREE").as_ptr(),
                0x10,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_launch_opens_window() {
        assert!(should_show(["TTREE.exe"]));
    }

    #[test]
    fn background_launch_stays_hidden() {
        assert!(!should_show(["TTREE.exe", "--hidden"]));
    }

    #[test]
    fn explicit_show_overrides_background_in_either_order() {
        assert!(should_show(["TTREE.exe", "--hidden", "--show"]));
        assert!(should_show(["TTREE.exe", "--show", "--hidden"]));
    }
}
