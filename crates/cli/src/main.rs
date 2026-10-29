fn main() {
    match ironsetup::cli_main(std::env::args().skip(1).collect()) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("IronSetup: {error:#}");
            std::process::exit(2);
        }
    }
}
