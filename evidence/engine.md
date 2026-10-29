# Engine author verification

Task: AC3 transactional installation, recovery, safe uninstallation, selections,
and native Windows integration. Actor: `/root/engine`, role `rust_engineer`.
Base: `82d6d0b`; shared core contract update cherry-picked from `b93de1d`.
Engine implementation: `8d26224`; Windows lifecycle fixture: `1d9e543`;
transaction-lock regression: `b9d5be5`. Independent review fixes: `8e37d46`.
Validated installed-identity API for bootstrap: `9726110`.

## Executed checks

| Check | Environment | Result | Evidence |
|---|---|---|---|
| `cargo test --offline -p ironsetup-engine -- --test-threads=1` | Linux x86_64, Rust 1.90.0 | PASS, exit 0 | 17 lifecycle tests plus 2 independent reviewer regressions; no ignored tests |
| `cargo clippy --offline -p ironsetup-engine --all-targets -- -D warnings` | Linux x86_64 | PASS, exit 0 | Fresh target directory, `CARGO_INCREMENTAL=0`, after the independent review fixes |
| `cargo check --offline -p ironsetup-engine --target x86_64-pc-windows-gnu` | Linux cross target | PASS, exit 0 | Native API modules compile |
| `cargo clippy --offline -p ironsetup-engine --target x86_64-pc-windows-gnu --all-targets -- -D warnings` | Linux cross target | PASS, exit 0 | Fresh target directory, after review fixes, including native registry/shortcut/ARP test source |
| Native Windows runtime | Windows | NOT_RUN by this actor | Cross compilation is not native runtime verification |
| `windows_lifecycle` under Wine | Wine 9.0 x64, Ubuntu package 9.0~repack-4build3 | PASS, exit 0 | 1 native-API lifecycle test passed in 2.46 s |

The toolchain and Cargo cache are isolated under the task workspace. No external
repository, hosted CI, deployment, or publication was performed.

## Lifecycle cases exercised

Fresh install, SHA-256 verification, removal of installed files while preserving
unowned files; restoration of preexisting overwritten files; preservation of
user edits plus original backups; rejection of corrupt payloads before writes;
rollback after a conflicting destination appears mid-transaction; skipping
preexisting optional files; recovery of a persisted incomplete journal;
refusal of a repeated installation; fixed-component and task dependencies;
traversal, case collisions, symlink and tampered-journal rejection; retained
files; `Uninstallable=no`; post-install argument passing without shell
evaluation; and exclusive metadata locks.

The Windows fixture uses a unique HKCU key and unique per-user Start Menu group.
It checks native COM `.lnk` creation, ARP registration, uninstaller creation,
prior registry value restoration, shortcut deletion, and directory cleanup.
It has a cleanup guard and does not write HKLM or the desktop.

## Intentional v0.1 limits

- Reinstall/upgrade at an existing IronSetup directory is rejected. Uninstall
  first; there is no implicit upgrade claim.
- Install files are confined to the chosen application directory. Shortcuts are
  limited to the current user's Start Menu or desktop roots.
- Registry support is HKCU/HKLM/HKA with string, expandsz, DWORD, QWORD, and binary
  values. HKLM requires administrator mode. Missing `uninsdeletevalue` retains
  the value during uninstall; rollback always restores the captured prior state.
- Edited files are preserved. If an overwritten original must also be retained,
  `.ironsetup` metadata and backups remain and the report explains their path.
- Arbitrary post-install programs run after the file/registry transaction
  commits. Nonzero exit status becomes a warning; their external effects are
  not represented as reversible installer operations.
- `verify_install` reports integrity of files and shortcuts. Registry values are
  not included in its `intact` result; this limitation is included in warnings.
- Transaction recovery is intended for interrupted process operations. Native
  power-loss and filesystem durability behavior has not been certified.
- Running uninstaller self-removal requires the caller's temporary re-launch
  mechanism, implemented by the root CLI/stub owner.

## Findings fixed during author verification

The first rollback regression originally encountered an existing path conflict
during preflight. The fixture now creates the conflict through the progress
callback after the first replacement, proving actual rollback behavior.
Windows module re-export visibility and formatting-only Clippy errors were
corrected; all reported successful checks refer to the repaired source.

Independent review and integrated binary acceptance belong to the root team.

## Independent findings and fixes

The compiler agent reproduced two engine defects without modifying production
source: a tampered journal could redirect an external file record to a
non-shortcut file; and a same-content replacement could leave original Unix
0600 permissions at 0644 after uninstall. Both tests failed before the fix.
`8e37d46` restricts external file records to Windows `.lnk` locations, refuses
external journal roots on non-Windows, and restores captured permissions even
when file bytes already equal the original. Both tests pass after the fix; the
reviewer also independently reran them in a fresh target directory.

An incremental build returned an old library artifact during verification, so
the repaired source was rebuilt in a new target directory with
`CARGO_INCREMENTAL=0`. All 19 Linux tests and both target Clippy checks passed
against that rebuilt source.

## Wine native-API execution

The dedicated Windows test binary built against repaired engine `8e37d46`
has SHA-256:

`f58e0321567cac2106a6007cd66c48d2342fe9ef3c2967fe16834c4ef15de10f`

It ran with `DISPLAY=:96`, a disposable task-owned `WINEPREFIX`,
`WINEARCH=win64`, and `WINEDEBUG=-all` through `/usr/lib/wine/wine64`, with
`--test-threads=1 --nocapture`. This test used native Win32 registry and COM
ShellLink calls under Wine. It is not a native Windows OS or hardware test.

Actual result:

```text
running 1 test
test native_registry_shortcut_and_arp_install_uninstall ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.46s
```

The test checked HKCU write/previous-value restoration, `.lnk` header and
deletion, ARP add/remove, uninstaller creation, and complete cleanup of the
application directory and unique Start Menu group. The Wine prefix was returned
to the desktop agent after completion. The later `installed_app_info` read-only
helper does not change these exercised installation/uninstallation paths.
