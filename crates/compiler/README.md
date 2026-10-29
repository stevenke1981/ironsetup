# IronSetup compiler v0.1.0

Rust implementation of an explicit Inno Setup script subset. It returns a `Package` in memory; writing packages and self-extracting executables belongs to the CLI/core crates. It does not invoke ISCC, use Inno Setup source code, evaluate Pascal, execute macros, or fetch remote files.

## API

```rust,ignore
let compiled = ironsetup_compiler::compile_script(
    std::path::Path::new("example.iss"),
    &ironsetup_compiler::CompileOptions::default(),
)?;
ironsetup_core::write_package_file(
    std::path::Path::new("example.ispkg"),
    &compiled.package,
)?;
```

`CompiledScript` provides `package`, `output_dir`, `output_basename`, and `warnings`. `preprocess` exposes the expanded text without compiling. Failures identify the originating script/include and line when an input line is responsible. Missing required directives and aggregate payload conflicts can have an overall script diagnostic.

## Supported grammar

- UTF-8 scripts with an optional BOM; CRLF or LF; whole-line `;` comments.
- Case-insensitive sections, directive names, parameter names, and flags.
- Entry values use `Name: value; Name: "quoted value"`; `""` encodes a quote inside a quoted value. Semicolons inside quotes are preserved.
- `#define NAME "literal"` or a single simple numeric/word literal; `{#NAME}` replacement; quoted `#include` paths resolved relative to the including script. CLI definitions override script definitions. Macro names are case-sensitive. Other ISPP directives and expressions fail explicitly.
- Includes are bounded to 32 levels and checked for cycles. Combined input is limited to 8 MiB, expanded text to 16 MiB / 200,000 lines.

## Sections and exact surface

| Section | Supported values |
| --- | --- |
| `[Setup]` | `AppId`, `AppName`, `AppVersion`, `AppPublisher`, `DefaultDirName`, `DefaultGroupName`, `PrivilegesRequired`, `Uninstallable`, `Compression`, `SolidCompression=no`, `OutputDir`, `OutputBaseFilename`, `SourceDir`, `LicenseFile`, `InfoBeforeFile`, `InfoAfterFile`, `ArchitecturesAllowed=x64compatible` |
| `[Files]` | `Source`, `DestDir`, `DestName`, `Excludes`, `Flags`, `Components`, `Tasks` |
| `[Dirs]` | `Name`, `Components`, `Tasks` |
| `[Types]` | `Name`, `Description`, `Flags: iscustom` |
| `[Components]` | `Name`, `Description`, `Types`, `Flags: fixed` |
| `[Tasks]` | `Name`, `Description`, `Components`, `Flags: unchecked` |
| `[Icons]` | `Name`, `Filename`, `Parameters`, `WorkingDir`, `Comment`, `IconFilename`, `Components`, `Tasks` |
| `[Registry]` | `Root`, `Subkey`, `ValueType`, `ValueName`, `ValueData`, `Flags`, `Components`, `Tasks` |
| `[Run]` | `Filename`, `Parameters`, `WorkingDir`, `Description`, `Flags`, `Components`, `Tasks` |

Required setup directives are `AppName`, `AppVersion`, and `DefaultDirName`. `AppId` defaults to `AppName`, and `DefaultGroupName` defaults to `AppName`. `OutputDir` defaults to `Output` and relative paths resolve against `SourceDir`; `OutputBaseFilename` defaults to `setup`. `SourceDir` itself resolves against the main script directory.

**Explicit differences:** `PrivilegesRequired` defaults to `lowest`, and compression defaults to `zip` (deflate). The compiler emits warnings when these defaults are implicit. Set both directives explicitly in production scripts. `Compression=none` is also supported; LZMA/LZMA2, solid compression, and compression-level suffixes are rejected.

### Files and directories

Every `[Files]` entry must explicitly include `ignoreversion` or `onlyifdoesntexist`. PE version comparison is not implemented, so implicit Inno Setup replacement semantics are not accepted. Other accepted flags are `uninsneveruninstall`, `recursesubdirs`, `createallsubdirs`, and `skipifsourcedoesntexist`. The latter three affect compilation and are removed from the runtime file flags.

Files and explicit directories must target `{app}` or its descendants. Source filename patterns support `*` and `?`; `*.*` includes extensionless files. Wildcards in source directory segments are rejected. Matching is case-insensitive. `Excludes` follows suffix matching, with a leading backslash anchoring a pattern at the source-tree root; unlike source `*.*`, exclude `*.*` requires a dot. Brackets are literal rather than glob character classes. Source wildcard support is a documented subset, not full Windows DOS wildcard equivalence.

Recursion preserves relative subdirectories; `createallsubdirs` additionally retains empty directories and requires `recursesubdirs`. Symlinks and Windows reparse points are rejected. Duplicate destination names are rejected case-insensitively, even if their selections appear disjoint. `DestName` can rename only one matched file. Traversal is bounded to 128 levels / 200,000 visited entries; packages are limited to 100,000 files and 512 MiB uncompressed payload.

### Selections

Component/task names use ASCII letters, digits, `_`, `-`, and `.`. Names are normalized to lowercase. Boolean expressions support `not`, `and`, `or`, parentheses, and whitespace-separated OR lists; references are validated. Hierarchical component/task names, parent-child inheritance flags, exclusive groups, task-to-task conditions, and component-to-component conditions are rejected. One `iscustom` type is permitted.

### Windows integration

Icon destinations start with `{group}`, `{userdesktop}`, `{autodesktop}`, or `{userprograms}`. Targets must be app-local or exactly `{uninstallexe}`; the latter requires `Uninstallable=yes`. Working directory and icon file must be app-local. Nested constants in icon names are rejected.

Registry roots are `HKCU`, `HKLM`, or `HKA`; explicit `HKLM` requires `PrivilegesRequired=admin`. Types are `string`, `expandsz`, `dword`, `qword`, and `binary`. The only accepted flag is `uninsdeletevalue`. Integers accept decimal, `$HEX`, or `0xHEX` and are normalized to decimal; binary accepts hexadecimal pairs with optional whitespace. Subkeys and value names are literal. Duplicate value destinations are rejected. Runtime rollback/restoration behavior is specified by the engine.

Run targets must be app-local. Flags are `skipifsilent`, `runhidden`, `nowait`, and `waituntilterminated`; `nowait` conflicts with `waituntilterminated`. `nowait` reports launch success only and emits a compiler warning. Arbitrary child program side effects cannot be rolled back. `postinstall`, `shellexec`, and `[UninstallRun]` are not supported.

Supported runtime constants are `{app}`, `{group}`, `{userdesktop}`, `{autodesktop}`, `{userprograms}`, `{localappdata}`, `{userappdata}`, `{userpf}`, `{autopf}`, `{pf}`, `{pf64}`, and `{uninstallexe}`. A literal opening brace in supported dynamic strings uses `{{`. The allowed subset is narrower for default install paths, file paths, shortcuts, and run targets. Unknown constants, including `{code:...}`, fail explicitly.

Text documents are embedded as UTF-8 plain text, at most 4 MiB each; RTF is explicitly rejected. The `userdocs:` output-path prefix, runtime downloads, services, encryption, signing directives, languages/messages sections, and Pascal `[Code]` are not implemented.

## Evidence and upstream references

The crate's integration tests cover deterministic package encoding/decoding, parser edge cases, includes/overrides, strict diagnostics, path safety, recursion/exclusion behavior, component expressions, registry normalization, and executable-action restrictions. A compiler test passing does not establish Windows installer runtime correctness.

Official reference pages consulted on 2026-10-25 (Asia/Taipei):

- <https://jrsoftware.org/ishelp/topic_filessection.htm>
- <https://jrsoftware.org/ishelp/topic_setup_outputdir.htm>
- <https://jrsoftware.org/ishelp/topic_setup_sourcedir.htm>
- <https://jrsoftware.org/ishelp/topic_compilercmdline.htm>
- <https://jrsoftware.org/ishelp/topic_componentssection.htm>
- <https://jrsoftware.org/ishelp/topic_runsection.htm>

Only public behavior documentation was used; this crate contains an independent implementation.
