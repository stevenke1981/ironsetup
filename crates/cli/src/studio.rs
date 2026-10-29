#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    let initial = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    if let Err(e) = ironsetup::studio_main(initial) {
        ironsetup_ui::show_error(&format!("{e:#}"));
        std::process::exit(1);
    }
}
