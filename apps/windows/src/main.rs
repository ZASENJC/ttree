//! TTREE native Win32 entry point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(target_os = "windows")]
mod app;
#[cfg(any(target_os = "windows", test))]
mod design;
#[cfg(target_os = "windows")]
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
#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("Build ttree-windows on Windows.");
}
