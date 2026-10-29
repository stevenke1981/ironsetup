# Desktop UI implementation and verification

Owner: `/root/desktop` (Agent Team Workflow `desktop_engineer`).
Worktree: `ironsetup-ui`, branch `feat/ui`. Scope: `crates/ui` and this evidence.
Baseline: `82d6d0b`; root's shared-core implementation `b93de1d` was integrated before checks.

## Delivered behavior

- Native Windows installation wizard: welcome, optional information, mandatory license acceptance when a license is present, destination browser, setup types, component/task selection, review, real worker progress, complete/error result.
- Required components remain selected. Tasks whose component conditions are false are deselected. The host can inject authoritative engine selections with `installer_wizard_with_selections`.
- The license gate is enforced both by the button's disabled state and the command handler. Closing is blocked while the installation callback runs.
- Worker callbacks use channels; HWNDs and UI state remain on the UI thread. The host keeps state alive until all child windows are destroyed. Worker panics become explicit errors.
- Native compiler studio: UTF-8 editor, New/Open/Save/Save As, Ctrl+N/O/S, F9, threaded build callback, full diagnostics/warnings, script folder opening. Saving stages and syncs a sibling file before replacement, and prompts if another program changed the loaded source.
- `WizardError::Cancelled` and `WizardError::OperationFailed` distinguish expected cancellation and already displayed failures from window startup errors. `show_information` provides a normal information dialog for help/uninstall outcomes.

## Executed checks

| Check | Platform | Result | Evidence |
|---|---|---|---|
| `cargo test --offline -p ironsetup-ui` | Linux x86_64, Rust 1.90.0 | PASS, 4 tests | Wizard route, default selections, Unicode newline preservation, typed outcomes |
| `cargo clippy --offline --target x86_64-pc-windows-gnu -p ironsetup-ui --tests -- -D warnings` | Windows cross-check | PASS, exit 0 | Native UI, test harness, and library typechecked without warnings |
| `cargo test --offline --target x86_64-pc-windows-gnu -p ironsetup-ui --test native_ui_smoke --no-run` | MinGW x86_64 cross-link | PASS, exit 0 | Real Windows PE test executable |
| `native_ui_smoke-*.exe --ignored --test-threads=1 --nocapture` | Wine 9.0 x64, Xvfb 1280×900 | PASS, 1 scenario, exit 0 | `wine-ui-smoke.log` |
| Internal visual inspection | Wine/Xvfb | PASS for layout and visible controls | License, selection, result, and studio windows inspected; images remain temporary and are not shipped |
| Native Windows 10/11 execution | Actual Windows host | NOT_RUN | No native Windows machine was connected |

The opt-in native GUI smoke test drives only windows owned by its own process. It verifies the direct-command license bypass is rejected; accepted license; typed destination with spaces; setup types; fixed components; conditional tasks; progress; closing blocked during work; completion/warning text; UTF-8 editor save; F9 build; and build close blocking. The injected callbacks are test fixtures, **not real application installation or compilation**. The separate engine lifecycle/integration checks cover those layers.

The initial screenshot exposed a page repaint defect: a background erase could cover the license checkbox. The parent window now uses `WS_CLIPCHILDREN`, and page transitions explicitly invalidate visible children. The GUI test was rerun after this fix. A fresh screenshot confirmed the checkbox and label were visible, while the license gate still passed.

## Environment repair

The relocated Wine package initially failed to load `kernel32.dll`. Setting `WINEDLLPATH` confirmed a relocation/data-path problem, but the incomplete prefix lacked normal Windows folders. Official Ubuntu `wine64` 9.0 was then installed in the disposable Linux runtime and a fresh project-specific prefix initialized. This did not access or modify the user's computer. Wine's x64 runner successfully loaded and executed both the GUI test and, in the separate engine agent's work, the native Windows integration test.

Runtime invocation used `/usr/lib/wine/wine64`, `DISPLAY=:96`, `WINEARCH=win64`, and a task-specific `WINEPREFIX`. Native Wine loader paths required no overrides after the standard installation.

## Limits

- Wine execution is not native Windows validation. UAC behavior, native Windows themes, high contrast interactions, monitor changes, screen readers, and Windows installer reputation were not validated here.
- UI text is English in v0.1. Scripts and filenames use Unicode. The Wine test environment lacks CJK font glyphs, so the screenshot renders Chinese fixture text as missing-glyph boxes; exact Unicode native-control text and UTF-8 file round-trip assertions passed.
- The host scales native controls/fonts to the system DPI at startup. Live per-monitor DPI changes are not implemented.
- The studio is a plain native source editor; it does not claim syntax highlighting, a Pascal debugger, or a full Inno Setup IDE clone.
- The file and folder dialogs are implemented and cross-compiled. Their interactive workflows were not automated by this smoke test.

## API references used

- Microsoft: [SetWindowLongPtrW / GWLP_USERDATA](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowlongptrw)
- Microsoft: [OPENFILENAMEW](https://learn.microsoft.com/en-us/windows/win32/api/commdlg/ns-commdlg-openfilenamew)
- Microsoft: [EM_SETLIMITTEXT](https://learn.microsoft.com/en-us/windows/win32/controls/em-setlimittext)
- Ubuntu: [Wine environment variables](https://manpages.ubuntu.com/manpages/noble/man1/wine-stable.1.html)
