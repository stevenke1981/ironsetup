#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let silent = args.iter().any(|a| {
        matches!(
            a.to_ascii_lowercase().as_str(),
            "/silent" | "/verysilent" | "--silent" | "--very-silent"
        )
    });
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
