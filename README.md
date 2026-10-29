# IronSetup 0.1.0

A standalone Rust installer builder with a strict, documented subset of Inno Setup's `.iss` authoring workflow. Includes a native Win32 compiler studio and installer wizard, verified packages, transactional file installation, modification-aware uninstall, Windows shortcuts/registry/ARP integration, CLI tools, examples and actual validation evidence.

Start with **[README.zh-TW.md](README.zh-TW.md)** for the complete user guide. On Windows, extract the portable ZIP and run **Start-IronSetup.cmd** or **Build-Demo.cmd**. No Rust installation is needed for the prebuilt binaries.

This is v0.1.0, not a full Inno Setup replacement. Unsupported syntax is rejected. Pascal scripts, in-place upgrades, LZMA, Authenticode publishing and multilingual UI are outside this release. Native Windows acceptance is distinguished from Wine and cross-compilation in `evidence/VERIFICATION.md`.

```sh
cargo test --locked --workspace
cargo build --locked --release --workspace
python3 scripts/smoke.py --bin-dir target/release
```

MIT licensed independent implementation. See `docs/COMPATIBILITY.md`, `docs/ARCHITECTURE.md`, `docs/dispatch-plan.md`, and `THIRD_PARTY_NOTICES.md`.
