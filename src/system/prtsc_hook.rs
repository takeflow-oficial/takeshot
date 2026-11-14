//! PrtSc priority on Windows: a low-level keyboard hook that takes PrtSc before other apps.

// The hook exists only on Windows; elsewhere only the tests use its rules.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::system::hotkeys::Shot;

/// Windows' virtual-key code for Print Screen.
const VK_SNAPSHOT: u32 = 0x2C;

/// Which capture a key event starts: a fresh press (not auto-repeat) of PrtSc or Shift+PrtSc. With
/// Win, Alt or Ctrl held the key stays Windows' own (Win+PrtSc saves, Alt+PrtSc copies the window).
pub fn hook_decision(
    vk: u32,
    key_down: bool,
    shift: bool,
    other_mods: bool,
    already_down: bool,
) -> Option<Shot> {
    if !swallows(vk, other_mods) || !key_down || already_down {
        return None;
    }
    Some(if shift { Shot::Full } else { Shot::Area })
}

/// The hook hides PrtSc and Shift+PrtSc, press and release, from other apps (the Snipping Tool can
/// act on the release); everything else, Win/Alt/Ctrl+PrtSc included, passes through.
pub fn swallows(vk: u32, other_mods: bool) -> bool {
    vk == VK_SNAPSHOT && !other_mods
}

#[cfg(windows)]
pub use imp::PrtScHook;

#[cfg(windows)]
mod imp {
    use super::{hook_decision, swallows};
    use crate::system::hotkeys::Shot;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, PeekMessageW,
        PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN,
        WM_QUIT, WM_SYSKEYDOWN,
    };

    type Sink = Arc<dyn Fn(Shot) + Send + Sync>;

    /// The hook procedure is a plain function, so it reaches the callback through a global.
    static SINK: Mutex<Option<Sink>> = Mutex::new(None);
    /// PrtSc is held down: its auto-repeat must not start more captures.
    static DOWN: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            // SAFETY: for WH_KEYBOARD_LL with code >= 0, lparam points to a KBDLLHOOKSTRUCT.
            let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            // SAFETY: plain Win32 calls without pointers.
            let held = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) } < 0;
            let other = held(VK_CONTROL) || held(VK_MENU) || held(VK_LWIN) || held(VK_RWIN);
            if swallows(kb.vkCode, other) {
                let down = wparam == WM_KEYDOWN as WPARAM || wparam == WM_SYSKEYDOWN as WPARAM;
                let was_down = DOWN.swap(down, Ordering::Relaxed);
                if let Some(shot) = hook_decision(kb.vkCode, down, held(VK_SHIFT), other, was_down)
                {
                    let sink = SINK.lock().ok().and_then(|s| s.clone());
                    if let Some(sink) = sink {
                        sink(shot);
                    }
                }
                return 1;
            }
        }
        // SAFETY: forwards the event unchanged to the next hook.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    /// The installed hook; dropping it removes the hook and ends its thread.
    pub struct PrtScHook {
        thread_id: u32,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl PrtScHook {
        /// Installs the hook on its own thread with a message loop, as low-level hooks require.
        /// `on_shot` must return quickly: Windows drops hooks that stall input. Only one hook may
        /// live at a time (its callback is process-wide); a second install fails with `AlreadyExists`.
        pub fn install(on_shot: impl Fn(Shot) + Send + Sync + 'static) -> std::io::Result<Self> {
            {
                let mut sink = SINK
                    .lock()
                    .map_err(|_| std::io::Error::other("hook state poisoned"))?;
                if sink.is_some() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "the PrtSc hook is already installed",
                    ));
                }
                *sink = Some(Arc::new(on_shot));
            }
            let installed = Self::start_thread();
            if installed.is_err()
                && let Ok(mut s) = SINK.lock()
            {
                *s = None;
            }
            installed
        }

        fn start_thread() -> std::io::Result<Self> {
            let (tx, rx) = channel();
            let thread = std::thread::Builder::new()
                .name("takeshot-prtsc-hook".into())
                .spawn(move || {
                    // SAFETY: Win32 calls on this thread; the hook handle never leaves it.
                    unsafe {
                        let module = GetModuleHandleW(std::ptr::null());
                        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0);
                        if hook.is_null() {
                            let _ = tx.send(Err(std::io::Error::last_os_error()));
                            return;
                        }
                        // Creates this thread's message queue before Drop may post WM_QUIT to it.
                        let mut msg: MSG = std::mem::zeroed();
                        PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                        let _ = tx.send(Ok(GetCurrentThreadId()));
                        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
                        UnhookWindowsHookEx(hook);
                    }
                })?;
            let thread_id = rx
                .recv()
                .map_err(|_| std::io::Error::other("hook thread ended"))??;
            Ok(Self {
                thread_id,
                thread: Some(thread),
            })
        }
    }

    impl Drop for PrtScHook {
        fn drop(&mut self) {
            // SAFETY: posts WM_QUIT to the hook thread's own message queue.
            let posted = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) } != 0;
            if let Some(t) = self.thread.take() {
                if posted {
                    let _ = t.join();
                } else {
                    // Without the quit message the thread would never end: leave it rather than hang.
                    crate::system::log::line(format_args!("could not stop the PrtSc hook thread"));
                }
            }
            if let Ok(mut s) = SINK.lock() {
                *s = None;
            }
            DOWN.store(false, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VK_A: u32 = 0x41;

    #[test]
    fn the_hook_fires_on_a_fresh_prtsc_press_only() {
        let fire = |vk, down, shift, other, held| hook_decision(vk, down, shift, other, held);
        assert_eq!(
            fire(VK_SNAPSHOT, true, false, false, false),
            Some(Shot::Area)
        );
        assert_eq!(
            fire(VK_SNAPSHOT, true, true, false, false),
            Some(Shot::Full)
        );
        assert_eq!(fire(VK_SNAPSHOT, false, false, false, true), None, "key up");
        assert_eq!(
            fire(VK_SNAPSHOT, true, false, false, true),
            None,
            "held down: auto-repeat"
        );
        assert_eq!(fire(VK_A, true, false, false, false), None);
        // Win+PrtSc saves a screenshot, Alt+PrtSc copies the window: those stay Windows'.
        assert_eq!(
            fire(VK_SNAPSHOT, true, false, true, false),
            None,
            "Win/Alt/Ctrl held"
        );
    }

    #[test]
    fn the_hook_swallows_plain_prtsc_events_and_nothing_else() {
        assert!(swallows(VK_SNAPSHOT, false));
        assert!(
            !swallows(VK_SNAPSHOT, true),
            "Win/Alt/Ctrl+PrtSc pass through"
        );
        assert!(!swallows(VK_A, false));
    }

    #[cfg(windows)]
    #[test]
    fn the_hook_installs_once_and_uninstalls() {
        let hook = PrtScHook::install(|_| {}).expect("install the keyboard hook");
        // A second hook would be left without a callback when the first one drops.
        let again = PrtScHook::install(|_| {});
        assert_eq!(
            again.err().map(|e| e.kind()),
            Some(std::io::ErrorKind::AlreadyExists)
        );
        drop(hook);
        let hook = PrtScHook::install(|_| {}).expect("install again after the drop");
        drop(hook);
    }
}
