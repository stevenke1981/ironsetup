IronSetup Demo / Rust installer example

The compiler reads .iss scripts, packs local files, and attaches a verified
compressed payload to a Rust installer stub.

The Windows example creates Start Menu shortcuts and an Installed Apps entry.
The desktop shortcut is optional. The config/settings.toml file is installed
only when it does not already exist.

After installation, edit a payload file and run uninstall: your modified file
is preserved. Unrelated user files are never recursively deleted with the app.

繁體中文檔案內容測試：這個檔案由 Rust 安裝引擎寫入。
