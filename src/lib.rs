pub mod build;
pub mod cache;
pub mod cli;
pub mod controls;
pub mod firmware;
pub mod lamps;
pub mod link;
pub mod media;
pub mod ocr;
pub mod package;
pub mod qmp;
pub mod runtime;
pub mod shell;
pub mod state;
pub mod util;
pub mod validation;
use std::sync::atomic::{AtomicBool, Ordering};
static STOPPED: AtomicBool = AtomicBool::new(false);
pub fn stopped() -> bool {
    STOPPED.load(Ordering::Relaxed)
}
extern "C" fn on_signal(_: libc::c_int) {
    STOPPED.store(true, Ordering::Relaxed);
}
pub fn install_signals() {
    // SAFETY: the handler has the required C ABI and only updates a lock-free atomic flag.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
}
