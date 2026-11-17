#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Also true for -VERYSILENT and a Git Bash-mangled /VERYSILENT, so a
    // scripted run never blocks on a modal error dialog.
    let silent = ironsetup::wants_silent(&args);
    match ironsetup::stub_main(args) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            let message = format!("{error:#}");
            if !silent {
                ironsetup_ui::show_error(&message);
            }
            eprintln!("IronSetup: {message}");
            std::process::exit(1);
        }
    }
}
