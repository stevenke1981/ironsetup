# IronSetup working agreement

User objective: independently implement an Inno Setup inspired Windows installer builder in Rust and deliver a ZIP. This is v0.1.0, an executable compatibility subset, not a claim of full Inno Setup compatibility.

Use Agent Team Workflow. Every writer has a separate Git worktree and explicit crate/file ownership. Root owns shared model and integration. Do not modify another owner's files or shared Cargo.lock. Keep unknown directives/flags fail-closed with source line diagnostics. Do not silently accept unimplemented features. Do not use or invoke Inno Setup binaries/source code. Do not publish externally.

Acceptance: compile an .iss file to a verified package and Windows self-extracting EXE; install/uninstall with safe paths and rollback; Windows native wizard and compiler studio; real cross-platform tests and a Windows build when the environment permits; document anything not run. Never label Wine or cross-compilation as native Windows testing.

The original delivery used an isolated toolchain outside the repository. Future contributors should inspect the current environment and use an available Rust toolchain; do not assume the original workspace paths still exist. Agent source changes are committed to their own branches and integrated by root. Evidence records actual commands, exit status, target platform, and unresolved issues. Reviews focus on concrete correctness bugs, followed by fixes and targeted verification.
