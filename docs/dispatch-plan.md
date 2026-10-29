# IronSetup v0.1.0 dispatch plan

Date: 2026-10-25 (Asia/Taipei). Reference: official Inno Setup 7.1.0 documentation, accessed during this task.

| Owner | Worktree / write scope | Acceptance |
|---|---|---|
| root / integrator | ironsetup; core, CLI, build scripts, docs, examples, packaging | AC1 package integrity, AC5 executable distribution, AC6 evidence |
| compiler agent | ironsetup-compiler; crates/compiler only | AC2 strict ISS parsing, macros/includes, filesystem expansion, deterministic compile |
| engine agent | ironsetup-engine; crates/engine only | AC3 rollback, uninstall, selections, Windows integration |
| desktop agent | ironsetup-ui; crates/ui only | AC4 native Windows installer wizard + compiler studio |

Plan → independent crate implementation → local commits → root integration → targeted independent review + tests → fixes → ZIP. GitHub PR/hosted CI/public release are N/A: user requested a ZIP and did not request publication. A reusable local CI script is included; no hosted CI minutes were consumed.

Baseline scope: strict .iss subset; #define/#include; [Setup], [Files], [Dirs], [Types], [Components], [Tasks], [Icons], [Registry], [Run]; ZIP/deflate package, SHA-256 integrity; single-EXE payload trailer; native Windows installer/studio; per-user installation, selected components/tasks, rollback and modification-aware uninstall. Unsupported advanced Pascal [Code], full ISPP, LZMA, encryption, downloads, services and reboot replacement must fail explicitly.

All agents inherit the session model. No model or reasoning-effort switch is claimed.
