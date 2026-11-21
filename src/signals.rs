//! Signal handling. SIGINT/SIGTERM/SIGHUP/SIGQUIT set a flag instead of
//! killing duxdm mid-prompt: the UI loops poll `interrupted()`, cancel the
//! current prompt and exit cleanly — restoring the terminal on the way out.
//! Handlers are installed WITHOUT SA_RESTART so a blocked read() returns
//! EINTR immediately and the prompt reacts at once.

use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    // Lock-free atomic store: async-signal-safe.
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Install the flag-setting handlers.
pub fn install() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_signal as *const () as usize as libc::sighandler_t;
        sa.sa_flags = 0; // no SA_RESTART: blocked reads get EINTR
        libc::sigemptyset(&mut sa.sa_mask);
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

/// Has a signal arrived since the last check? Consumes the flag.
/// ^C at the login prompt cancels (getty-like: the service restarts
/// duxdm and the prompt comes back); SIGTERM from systemd makes the
/// main loop exit so the unit can stop cleanly.
pub fn interrupted() -> bool {
    INTERRUPTED.swap(false, Ordering::SeqCst)
}
