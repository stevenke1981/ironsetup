# Actual release Studio → SFX → GUI install verification

Date: 2026-10-25 (Asia/Taipei). Owner: `/root/desktop`.
Product source head: `29f5a44`, including compiler fix `7a6cdd0` for canonical Windows `\\?\` paths.

## Result

**PASS under Wine 9.0 x64/Xvfb, exit 0, using the actual release executables and real native Next buttons. Native Windows execution remains NOT_RUN.**

This check used no injected compiler or installer callbacks. The controller spawned the real `ironsetup-studio.exe`, found only the window belonging to that child PID, opened an isolated `.iss`, and pressed F9. Studio saved/canonicalized the script, invoked its real compiler, found its sibling `ironsetup-stub.exe`, and wrote a new 1,330,608-byte SFX. Its build log explicitly showed a `\\?\Z:\...` output path, exercising the canonical-path regression that prompted this check.

The actual `ironsetup.exe inspect` then decoded and verified the generated SFX. The controller launched that SFX, accepted its license, selected the isolated destination, and completed a real installation. The installed `hello.txt` bytes matched the source; `unins000.exe` existed. The actual CLI `verify` returned `intact: true`, and the actual CLI `uninstall` removed the isolated target. No existing product was installed over or removed.

Exact logs:

- `real-studio-control-integration.log`: complete successful chain using `BM_CLICK` on the real Next button and a 150 ms page-settling delay.
- `real-studio-integration.log`: first attempt, retained as failure evidence.
- `real-studio-release-hashes.json`: SHA-256 and size of all three actual release EXEs, generated SFX hash, and confirmation the release files did not change during testing.

## First attempt and its limit

The first controller directly posted `WM_COMMAND` for Next without allowing page painting/focus messages to settle. Studio F9 and SFX inspection passed, but after license acceptance the installer process terminated with Wine `free(): invalid pointer`, exit `0x80000101`. It had not installed files; the target did not exist.

The test controller was changed to exercise the real native Next button using `BM_CLICK`, with 150 ms between page transition and click. The complete chain then passed using the **same release hashes**, with **no production source change**. The first assertion's underlying Wine/native-message cause has not been proven. This evidence establishes the successful normal-button path under Wine; it does not establish native Windows correctness or diagnose every rapid synthetic-message path.

## Reproducible driver

`crates/ui/examples/real_studio_smoke.rs` is an opt-in Windows integration driver. It accepts five paths:

```text
real_studio_smoke.exe studio.exe ironsetup.exe script.iss output.exe temporary-install-dir
```

It requires a fresh output and a nonexistent temporary destination. The fixture script names the application `Studio Integration Smoke`, embeds a plain-text license, uses `PrivilegesRequired=lowest` and `Compression=zip`, and installs only `payload\hello.txt`. The required payload bytes are `Real Studio / real SFX / real install.` followed by a newline. This driver is intended for that isolated fixture, not arbitrary applications.

The helper cross-linked successfully and passed Windows-target clippy with `-D warnings`. Temporary installers, payloads, installation directories, and internal screenshots are not shipped as evidence artifacts.

## Release identity

| File | SHA-256 |
|---|---|
| `ironsetup.exe` | `11064d5c351720c92dc1a6714e9beaf0cd95b8361ac9e861e636da59f681dbf8` |
| `ironsetup-studio.exe` | `1674dc0bd34bf9aa63b714129f323395cc91bb6c8466b5434dbffe5e0d020271` |
| `ironsetup-stub.exe` | `690bb663b607851402d40bc2d7dae8638f7f85256ad81a3e2f78b6be556f687e` |

All three hashes were checked again after the test and matched.
