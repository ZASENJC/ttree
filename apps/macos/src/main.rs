//! TTREE macOS AppKit entry point.
#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod state;
#[cfg(target_os = "macos")]
mod ui_helpers;
#[cfg(target_os = "macos")]
mod shortcut_recorder;

#[cfg(target_os = "macos")]
fn main() {
    #[cfg(feature = "updater-smoke")]
    if let Some(result) = ttree_updater::smoke_from_args() {
        if let Err(error) = result {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
        return;
    }
    app::run();
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Build ttree-macos on macOS.");
}

#[cfg(target_os = "macos")]
pub(crate) fn dispatch_splitter_drag(id: u8, delta: f64) {
    app::on_splitter_drag(id, delta);
}
#[cfg(target_os = "macos")]
pub(crate) fn dispatch_splitter_drag_end() {
    app::on_splitter_drag_end();
}
