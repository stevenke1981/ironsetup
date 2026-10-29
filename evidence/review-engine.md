# Independent engine review — initial findings

Date: 2026-10-25 (Asia/Taipei). Reviewer instance: `/root/compiler`, acting as `independent_reviewer`, separate from the engine author. This review does not confer a human or GitHub approval authority.

Scope: engine commit `2642f32`, its filesystem journal/install/uninstall behavior, and core path-guard interaction. Reproduced in a separate worktree at `112169c` (a cherry-pick of the engine commit, based on the independently implemented compiler and core `b93de1d`). Production source was read only. Two regression tests were added only in the review worktree.

## REVIEW-ENGINE-01 — P1 — Journal accepts unrelated non-shortcut file locations outside the app

**Location:** `crates/engine/src/journal.rs`, `FileLocation::resolve` (34–44), `Journal::validate` (219–225), file removal (485).

**Trigger:** Install a normal app without shortcuts. Change a journal file record to `root: programs`, `path: private-notes.txt`, and set its installed digest to the bytes of a disposable unrelated file in the test's application-menu directory. Call `uninstall`.

**Observed result:** The call succeeds with an `UninstallReport.removed` containing the unrelated external file; that file is deleted. The uninstaller accepts any safe relative filename under `programs`/`desktop`, although the installer only creates `.lnk` files at those roots.

**Evidence:** `journal_cannot_redirect_uninstall_to_unrelated_nonshortcut_file` fails because the external file no longer exists. The test isolates the actual `XDG_DATA_HOME` configuration to a disposable directory, restores its prior value, and does not modify HOME or real user files.

**Fix direction:** Validate file record roles independently from directory record roles; external file records must be supported shortcut files, and external Windows actions must not be executed from Linux journals. Preserve the explicit distinction between path-confinement validation and authenticated metadata: these checks alone do not make a writable journal tamper-proof.

**Platform evidence:** Reproduced on Linux. The permissive path-resolution function is shared with Windows; native Windows behavior was not executed.

## REVIEW-ENGINE-02 — P2 — Equal original/installed bytes skip restoration of original permissions

**Location:** `crates/engine/src/journal.rs`, `Journal::undo`, `matches_original` branch (469–475).

**Trigger:** Create a preexisting `settings.txt` with mode `0600`; install the same bytes with the regular payload mode `0644`; uninstall.

**Observed result:** The original file remains mode `0644`. The `matches_original` branch assumes that equal bytes mean a recovery already restored the file, skips its recorded original permissions, and then permits backup/metadata cleanup. Content equality does not establish permission equality.

**Evidence:** `equal_content_original_file_permissions_are_restored` fails with actual decimal mode `420` (`0644`) versus expected `384` (`0600`).

**Fix direction:** Restore captured permissions in the equal-original-content branch, or preserve applicable original permissions when replacing the file. Verify both ordinary uninstall and the idempotent recovery path with equal payload/original content.

**Platform evidence:** Reproduced on Linux; no Windows-specific permission-restoration claim is made.

## Reproduction command

```sh
cargo test --offline -p ironsetup-engine --test review_regressions -- --test-threads=1
```

Working directory: `/workspace/scratch/81c4b06116b5/ironsetup-compiler`.

Result: **FAIL**, exit `101`, **0 passed / 2 failed**, no ignored tests. Isolated Rust 1.90.0 Linux toolchain supplied by the root agent. Regression file: `crates/engine/tests/review_regressions.rs`.

The findings were sent to the engine author and root integrator. This focused review stops at these concrete findings; follow-up validation should execute the same regressions against the fixes. Native Windows registry/COM lifecycle remains NOT_RUN in this review.


## Targeted fix validation — RESOLVED

Both baseline findings above are preserved as historical evidence. The engine author fixed them in `8e37d46d30f1b9c04a9874e468d8d1030e5e1eb3`; the reviewer applied that production-only commit as `493d7a969b32051b1001e6440082763fb16f22ff` and reran the **same two tests without changes**. No additional review scope was opened.

The revised code separates file-location validation from directory-location validation. External file records must end in `.lnk`, external Windows shortcut roots are rejected on non-Windows, and uninstaller file locations cannot masquerade as directory records. The equal-original-content recovery branch now reapplies captured original permissions.

To prevent reuse of incremental artifacts, the verification used a new target directory and disabled incremental compilation:

```sh
CARGO_INCREMENTAL=0 \
CARGO_TARGET_DIR=/workspace/scratch/81c4b06116b5/ironsetup-compiler/target-review-fixed \
cargo test --offline -p ironsetup-engine --test review_regressions -- --test-threads=1
```

Result: **PASS**, exit `0`, **2 passed / 0 failed**, no ignored tests. The unrelated external file is retained, and the equal-content original file returns to mode `0600` after uninstall.

- `REVIEW-ENGINE-01`: **RESOLVED**, same regression passed.
- `REVIEW-ENGINE-02`: **RESOLVED**, same regression passed.
- Native Windows registry/COM execution: **NOT_RUN** in this review; separate platform validation remains required.

This is a focused independent verification of the two reported fixes, not a claim that arbitrary writable journals are authenticated or that every Windows installation feature has been exercised.
