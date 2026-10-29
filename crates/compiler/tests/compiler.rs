use ironsetup_compiler::{compile_script, preprocess, CompileOptions};
use ironsetup_core::{decode_package, encode_package};
use std::{fs, path::Path};
use tempfile::TempDir;

fn fixture() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("payload")).unwrap();
    fs::write(d.path().join("payload/app.txt"), "application\n").unwrap();
    d
}
fn header() -> &'static str {
    "[Setup]\nAppName=Example\nAppVersion=1.0\nDefaultDirName={localappdata}\\Example\nPrivilegesRequired=lowest\nCompression=zip\n"
}
fn script(root: &Path, extra: &str) -> std::path::PathBuf {
    let path = root.join("test.iss");
    fs::write(&path, format!("{}{extra}", header())).unwrap();
    path
}
fn file_entry() -> &'static str {
    "[Files]\nSource: \"payload\\app.txt\"; DestDir: \"{app}\"; Flags: ignoreversion\n"
}
fn compile_error(extra: &str) -> String {
    let d = fixture();
    compile_script(&script(d.path(), extra), &CompileOptions::default())
        .unwrap_err()
        .to_string()
}

#[test]
fn minimal_compilation_and_round_trip_are_deterministic() {
    let d = fixture();
    let path = script(d.path(), file_entry());
    let first = compile_script(&path, &CompileOptions::default()).unwrap();
    assert_eq!(first.package.manifest.app.name, "Example");
    assert_eq!(first.package.manifest.files[0].path, "app.txt");
    assert_eq!(first.package.payloads[0], b"application\n");
    assert_eq!(first.output_basename, "setup");
    assert!(first.warnings.is_empty());
    assert!(!d.path().join("Output").exists());
    let a = encode_package(&first.package).unwrap();
    let b = encode_package(
        &compile_script(&path, &CompileOptions::default())
            .unwrap()
            .package,
    )
    .unwrap();
    assert_eq!(a, b);
    assert_eq!(decode_package(&a).unwrap().payloads, first.package.payloads);
}

#[test]
fn actual_wildcards_in_source_directory_components_are_rejected() {
    for source in [r"pay?load\app.txt", r"pay*load\app.txt"] {
        let error = compile_error(&format!(
            "[Files]\nSource: \"{source}\"; DestDir: \"{{app}}\"; Flags: ignoreversion\n"
        ));
        assert!(error.contains("not directory segments"), "{error}");
    }
}

#[cfg(windows)]
#[test]
fn canonical_windows_script_path_compiles_with_verbatim_prefix() {
    use std::path::{Component, Prefix};
    let d = fixture();
    let path = fs::canonicalize(script(d.path(), file_entry())).unwrap();
    assert!(
        matches!(path.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _)))
    );
    let compiled = compile_script(&path, &CompileOptions::default()).unwrap();
    assert_eq!(compiled.package.manifest.files[0].path, "app.txt");
    assert_eq!(compiled.package.payloads[0], b"application\n");
}

#[test]
fn quoted_semicolons_and_doubled_quotes_survive() {
    let d = fixture();
    fs::write(d.path().join("payload/a;b.txt"), "semicolon").unwrap();
    let path = script(d.path(), "[Tasks]\nName: desktop; Description: \"Create \"\"desktop\"\"; shortcut\"; Flags: unchecked\n[Files]\nSource: \"payload\\a;b.txt\"; DestDir: \"{app}\"; Flags: ignoreversion; Tasks: desktop;\n");
    let c = compile_script(&path, &CompileOptions::default()).unwrap();
    assert_eq!(
        c.package.manifest.tasks[0].description,
        "Create \"desktop\"; shortcut"
    );
    assert_eq!(c.package.manifest.files[0].path, "a;b.txt");
}

#[test]
fn includes_defines_and_cli_overrides_have_source_locations() {
    let d = fixture();
    fs::write(
        d.path().join("version.iss"),
        "\u{feff}#define Version \"1.0\"\n#define Name \"Default name\"\n",
    )
    .unwrap();
    let path = d.path().join("main.iss");
    fs::write(&path, format!("#include \"version.iss\"\n[Setup]\nAppName={{#Name}}\nAppVersion={{#Version}}\nDefaultDirName={{localappdata}}\\Example\n{}", file_entry())).unwrap();
    let mut opts = CompileOptions::default();
    opts.defines.insert("Name".into(), "Override name".into());
    let c = compile_script(&path, &opts).unwrap();
    assert_eq!(c.package.manifest.app.name, "Override name");
    assert_eq!(c.package.manifest.app.version, "1.0");
    assert!(preprocess(&path, &opts)
        .unwrap()
        .contains("AppName=Override name"));
    fs::write(
        d.path().join("version.iss"),
        "#define Version \"1.0\"\n[Code]\nbegin end;\n",
    )
    .unwrap();
    let error = compile_script(&path, &opts).unwrap_err().to_string();
    assert!(error.contains("version.iss:2:"), "{error}");
    assert!(error.contains("Pascal"), "{error}");
}

#[test]
fn include_cycles_and_undefined_macros_are_errors() {
    let d = fixture();
    fs::write(d.path().join("a.iss"), "#include \"b.iss\"\n").unwrap();
    fs::write(d.path().join("b.iss"), "#include \"a.iss\"\n").unwrap();
    let error = preprocess(&d.path().join("a.iss"), &CompileOptions::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("Include cycle"));
    fs::write(d.path().join("a.iss"), "AppName={#Missing}\n").unwrap();
    assert!(
        preprocess(&d.path().join("a.iss"), &CompileOptions::default())
            .unwrap_err()
            .to_string()
            .contains("Undefined preprocessor name")
    );
}

#[test]
fn unknown_syntax_fails_closed() {
    for (extra, expected) in [
        ("[Setup]\nPassword=secret\n", "Unsupported [Setup] directive password"),
        ("[Code]\n", "Unsupported section"),
        ("[INI]\n", "Unsupported section"),
        ("[Files]\nSource: \"payload/app.txt\"; DestDir: \"{app}\"; Flags: ignoreversion; Permissions: users-full", "Unsupported parameter permissions"),
        ("[Files]\nSource: \"payload/app.txt\"; DestDir: \"{app}\"; Flags: ignoreversion restartreplace", "Unsupported flag restartreplace"),
        ("#if 1\n", "Unsupported preprocessor directive"),
    ] {
        let error = compile_error(extra);
        assert!(error.contains(expected), "expected {expected}: {error}");
        assert!(error.contains("test.iss:"), "{error}");
    }
}

#[test]
fn explicit_overwrite_policy_is_required() {
    let error = compile_error("[Files]\nSource: \"payload/app.txt\"; DestDir: \"{app}\"\n");
    assert!(error.contains("PE file-version comparison"));
    let d = fixture();
    assert!(compile_script(
        &script(
            d.path(),
            "[Files]\nSource: \"payload/app.txt\"; DestDir: \"{app}\"; Flags: onlyifdoesntexist\n"
        ),
        &CompileOptions::default()
    )
    .is_ok());
}

#[test]
fn recursive_sources_exclusions_and_empty_directories() {
    let d = fixture();
    fs::create_dir_all(d.path().join("payload/docs/empty")).unwrap();
    fs::create_dir_all(d.path().join("payload/Temp")).unwrap();
    fs::write(d.path().join("payload/docs/readme.txt"), "docs").unwrap();
    fs::write(d.path().join("payload/docs/cache.tmp"), "cache").unwrap();
    fs::write(d.path().join("payload/Temp/private.txt"), "private").unwrap();
    let c = compile_script(&script(d.path(), "[Files]\nSource: \"payload\\*\"; DestDir: \"{app}\"; Excludes: \"*.tmp,\\Temp\\*\"; Flags: ignoreversion recursesubdirs createallsubdirs\n"), &CompileOptions::default()).unwrap();
    let names: Vec<_> = c
        .package
        .manifest
        .files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(names, ["app.txt", "docs/readme.txt"]);
    assert!(c
        .package
        .manifest
        .dirs
        .iter()
        .any(|d| d.path == "docs/empty"));
    assert!(c
        .package
        .manifest
        .files
        .iter()
        .all(|f| f.flags == ["ignoreversion"]));
}

#[test]
fn exclude_patterns_match_unanchored_suffixes_and_do_not_expand_star_dot_star() {
    let d = fixture();
    fs::create_dir_all(d.path().join("payload/sub/foo")).unwrap();
    fs::write(d.path().join("payload/sub/foo/bar"), "omit").unwrap();
    fs::write(d.path().join("payload/README"), "no extension").unwrap();
    let c = compile_script(&script(d.path(), "[Files]\nSource: \"payload\\*\"; DestDir: \"{app}\"; Excludes: \"*.*,foo\\bar\"; Flags: ignoreversion recursesubdirs\n"), &CompileOptions::default()).unwrap();
    assert_eq!(c.package.manifest.files.len(), 1);
    assert_eq!(c.package.manifest.files[0].path, "README");
}

#[test]
fn source_star_dot_star_includes_extensionless_files() {
    let d = fixture();
    fs::write(d.path().join("payload/README"), "no extension").unwrap();
    let c = compile_script(
        &script(
            d.path(),
            "[Files]\nSource: \"payload/*.*\"; DestDir: \"{app}\"; Flags: ignoreversion\n",
        ),
        &CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(c.package.manifest.files.len(), 2);
}

#[test]
fn brackets_are_literal_in_source_and_exclude_patterns() {
    let d = fixture();
    fs::write(d.path().join("payload/[test].txt"), "brackets").unwrap();
    let c = compile_script(
        &script(
            d.path(),
            "[Files]\nSource: \"payload/[test].txt\"; DestDir: \"{app}\"; Flags: ignoreversion\n",
        ),
        &CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(c.package.manifest.files[0].path, "[test].txt");
    let c = compile_script(&script(d.path(), "[Files]\nSource: \"payload/*\"; DestDir: \"{app}\"; Excludes: \"[test].txt\"; Flags: ignoreversion\n"), &CompileOptions::default()).unwrap();
    assert_eq!(c.package.manifest.files.len(), 1);
    assert_eq!(c.package.manifest.files[0].path, "app.txt");
}

#[test]
fn nested_icon_constants_and_special_output_prefixes_are_rejected() {
    assert!(
        compile_error("[Icons]\nName: \"{group}\\{app}\"; Filename: \"{app}\\app.exe\"\n")
            .contains("Nested runtime constants")
    );
    assert!(compile_error("[Setup]\nOutputDir=userdocs:Output\n").contains("userdocs:"));
}

#[test]
fn missing_source_is_skipped_only_with_explicit_flag() {
    assert!(compile_error(
        "[Files]\nSource: \"missing/*\"; DestDir: \"{app}\"; Flags: ignoreversion\n"
    )
    .contains("does not exist"));
    let d = fixture();
    let c = compile_script(&script(d.path(), "[Files]\nSource: \"missing/*\"; DestDir: \"{app}\"; Flags: ignoreversion skipifsourcedoesntexist\n"), &CompileOptions::default()).unwrap();
    assert!(c.package.manifest.files.is_empty());
}

#[test]
fn unsafe_destinations_and_reserved_names_are_rejected() {
    for path in [
        "{app}\\..\\escape",
        "{app}\\file:stream",
        "{app}\\CON",
        "{app}\\trailing.",
        "{app}\\.ironsetup",
        "{userdesktop}",
        "{app}suffix",
    ] {
        let error = compile_error(&format!(
            "[Files]\nSource: \"payload/app.txt\"; DestDir: \"{path}\"; Flags: ignoreversion\n"
        ));
        assert!(error.contains("test.iss:"), "{error}");
    }
}

#[test]
fn duplicate_destinations_are_detected_case_insensitively() {
    let error = compile_error("[Files]\nSource: \"payload/app.txt\"; DestDir: \"{app}\"; Flags: ignoreversion\nSource: \"payload/app.txt\"; DestDir: \"{app}\"; DestName: APP.TXT; Flags: ignoreversion\n");
    assert!(error.contains("Duplicate destination"));
}

#[test]
fn file_directory_conflicts_are_detected() {
    let error = compile_error("[Files]\nSource: \"payload/app.txt\"; DestDir: \"{app}\"; Flags: ignoreversion\nSource: \"payload/app.txt\"; DestDir: \"{app}\\app.txt\"; DestName: child; Flags: ignoreversion\n");
    assert!(error.contains("Conflicting file and directory"));
    let error = compile_error("[Files]\nSource: \"payload/app.txt\"; DestDir: \"{app}\"; Flags: ignoreversion\n[Dirs]\nName: \"{app}\\app.txt\"\n");
    assert!(error.contains("validation failed"));
}

#[test]
fn feature_conditions_are_parsed_and_reference_checked() {
    let d = fixture();
    let extra = "[Types]\nName: full; Description: Full\nName: custom; Description: Custom; Flags: iscustom\n[Components]\nName: main; Description: Main; Types: full; Flags: fixed\nName: docs; Description: Documents; Types: full\n[Tasks]\nName: desktop; Description: Desktop; Components: main; Flags: unchecked\n[Files]\nSource: \"payload/app.txt\"; DestDir: \"{app}\"; Flags: ignoreversion; Components: \"MAIN and not (docs)\"; Tasks: desktop\n";
    let c = compile_script(&script(d.path(), extra), &CompileOptions::default()).unwrap();
    assert_eq!(
        c.package.manifest.files[0].conditions.components,
        "main and not ( docs )"
    );
    assert!(c.package.manifest.types[1].custom);
    let error = compile_script(
        &script(d.path(), &extra.replace("Tasks: desktop", "Tasks: unknown")),
        &CompileOptions::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("Unknown tasks choice"));
    for invalid in ["main and", "main or )", "(main", "main && docs"] {
        assert!(compile_script(
            &script(d.path(), &extra.replace("MAIN and not (docs)", invalid)),
            &CompileOptions::default()
        )
        .is_err());
    }
}

#[test]
fn too_deep_conditions_are_rejected_without_recursing_unboundedly() {
    let d = fixture();
    let expression = format!("{}main{}", "(".repeat(100), ")".repeat(100));
    let path = script(d.path(), &format!("[Components]\nName: main; Description: Main\n[Files]\nSource: \"payload/app.txt\"; DestDir: \"{{app}}\"; Flags: ignoreversion; Components: \"{expression}\"\n"));
    assert!(compile_script(&path, &CompileOptions::default())
        .unwrap_err()
        .to_string()
        .contains("nesting"));
}

#[test]
fn unsupported_component_hierarchy_and_cycles_are_rejected() {
    assert!(
        compile_error("[Components]\nName: parent\\child; Description: Child\n")
            .contains("hierarchical")
    );
    assert!(
        compile_error("[Components]\nName: main; Description: Main; Components: main\n")
            .contains("Unsupported parameter")
    );
    assert!(
        compile_error("[Tasks]\nName: t; Description: Task; Tasks: t\n")
            .contains("Unsupported parameter")
    );
}

#[test]
fn registry_values_are_normalized_and_roots_validated() {
    let d = fixture();
    let extra = "[Registry]\nRoot: HKCU; Subkey: Software\\Example; ValueType: dword; ValueName: Number; ValueData: $FF; Flags: uninsdeletevalue\nRoot: HKCU; Subkey: Software\\Example; ValueType: binary; ValueName: Binary; ValueData: \"ab 00 CD\"\n";
    let c = compile_script(&script(d.path(), extra), &CompileOptions::default()).unwrap();
    assert_eq!(c.package.manifest.registry[0].value_data, "255");
    assert_eq!(c.package.manifest.registry[1].value_data, "AB00CD");
    assert!(
        compile_error(&extra.replace("HKCU", "HKLM")).contains("require PrivilegesRequired=admin")
    );
    assert!(compile_error(&extra.replace("$FF", "4294967296")).contains("out of range"));
    assert!(compile_error(&extra.replace("ab 00 CD", "ABC")).contains("byte pairs"));
    assert!(
        compile_error(&extra.replace("Software\\Example", "Software\\{app}")).contains("Subkey")
    );
}

#[test]
fn run_entries_are_explicit_and_conflicting_flags_fail() {
    let d = fixture();
    let extra = "[Run]\nFilename: \"{app}\\app.exe\"; Parameters: \"--data \"\"{app}\\data\"\"\"; WorkingDir: \"{app}\"; Flags: skipifsilent runhidden nowait\n";
    let c = compile_script(&script(d.path(), extra), &CompileOptions::default()).unwrap();
    assert_eq!(
        c.package.manifest.run[0].parameters,
        "--data \"{app}\\data\""
    );
    assert!(c
        .warnings
        .iter()
        .any(|w| w.message.contains("launch success")));
    assert!(
        compile_error(&extra.replace("nowait", "nowait waituntilterminated"))
            .contains("cannot be combined")
    );
    assert!(compile_error(&extra.replace("{app}\\app.exe", "cmd.exe"))
        .contains("must start with {app}"));
    assert!(compile_error(&extra.replace("nowait", "postinstall")).contains("Unsupported flag"));
}

#[test]
fn icons_require_supported_destinations_and_app_targets() {
    let d = fixture();
    let extra = "[Icons]\nName: \"{group}\\Example\"; Filename: \"{app}\\app.exe\"\nName: \"{group}\\Uninstall\"; Filename: \"{uninstallexe}\"\n";
    assert_eq!(
        compile_script(&script(d.path(), extra), &CompileOptions::default())
            .unwrap()
            .package
            .manifest
            .icons
            .len(),
        2
    );
    assert!(
        compile_error(&extra.replace("{group}", "{sys}")).contains("Unsupported runtime constant")
    );
    assert!(
        compile_error(&format!("[Setup]\nUninstallable=no\n{extra}"))
            .contains("requires Uninstallable=yes")
    );
}

#[test]
fn output_directory_is_relative_to_source_directory() {
    let d = fixture();
    let path = script(d.path(), "[Setup]\nSourceDir=payload\nOutputDir=artifacts\n[Files]\nSource: app.txt; DestDir: \"{app}\"; Flags: ignoreversion\n");
    let c = compile_script(&path, &CompileOptions::default()).unwrap();
    assert_eq!(
        c.output_dir,
        fs::canonicalize(d.path())
            .unwrap()
            .join("payload")
            .join("artifacts")
    );
}

#[test]
fn plain_utf8_license_is_embedded_and_rtf_rejected() {
    let d = fixture();
    fs::write(
        d.path().join("license.txt"),
        "\u{feff}授權條款\nLicense text",
    )
    .unwrap();
    let path = script(d.path(), "[Setup]\nLicenseFile=license.txt\n");
    let c = compile_script(&path, &CompileOptions::default()).unwrap();
    assert_eq!(
        c.package.manifest.license_text.as_deref(),
        Some("授權條款\nLicense text")
    );
    fs::write(d.path().join("license.txt"), "{\\rtf1 unsupported}").unwrap();
    assert!(compile_script(&path, &CompileOptions::default())
        .unwrap_err()
        .to_string()
        .contains("RTF rendering"));
}

#[test]
fn bad_quotes_and_duplicate_parameters_have_line_diagnostics() {
    assert!(compile_error("[Files]\nSource: \"unterminated\n").contains("test.iss:8: Unterminated"));
    assert!(compile_error("[Files]\nSource: a; Source: b\n").contains("Duplicate parameter source"));
    assert!(
        compile_error("[Files]\nSource: \"a\" trailing; DestDir: \"{app}\"\n")
            .contains("Unexpected characters")
    );
}

#[test]
fn destname_renames_single_file_and_rejects_colliding_multiple_sources() {
    let d = fixture();
    let extra = "[Files]\nSource: \"payload/*\"; DestDir: \"{app}\"; DestName: renamed.txt; Flags: ignoreversion\n";
    let c = compile_script(&script(d.path(), extra), &CompileOptions::default()).unwrap();
    assert_eq!(c.package.manifest.files[0].path, "renamed.txt");
    fs::write(d.path().join("payload/other.txt"), "other").unwrap();
    assert!(
        compile_script(&script(d.path(), extra), &CompileOptions::default())
            .unwrap_err()
            .to_string()
            .contains("DestName cannot rename multiple")
    );
}

#[cfg(unix)]
#[test]
fn symbolic_link_sources_are_rejected() {
    let d = fixture();
    std::os::unix::fs::symlink(
        d.path().join("payload/app.txt"),
        d.path().join("payload/link.txt"),
    )
    .unwrap();
    assert!(
        compile_script(&script(d.path(), file_entry()), &CompileOptions::default())
            .unwrap_err()
            .to_string()
            .contains("Symbolic links are not packaged")
    );
}

#[test]
fn runtime_constants_are_checked_and_literal_braces_are_preserved() {
    let d = fixture();
    let extra = "[Registry]\nRoot: HKCU; Subkey: Software\\Example; ValueType: string; ValueName: Data; ValueData: \"{{literal}; {APP}\"\n";
    let c = compile_script(&script(d.path(), extra), &CompileOptions::default()).unwrap();
    assert_eq!(
        c.package.manifest.registry[0].value_data,
        "{{literal}; {app}"
    );
    assert!(compile_error(&extra.replace("{APP}", "{code:Run}"))
        .contains("Unsupported runtime constant"));
}

#[test]
fn setup_rejects_unsupported_compression_architecture_and_default_roots() {
    let d = fixture();
    for (from, to, expected) in [
        ("Compression=zip", "Compression=lzma2", "LZMA/LZMA2"),
        (
            "DefaultDirName={localappdata}\\Example",
            "DefaultDirName={localappdata}",
            "application subdirectory",
        ),
        (
            "DefaultDirName={localappdata}\\Example",
            "DefaultDirName=C:\\",
            "Invalid relative path",
        ),
    ] {
        let path = d.path().join("test.iss");
        fs::write(&path, header().replace(from, to)).unwrap();
        let error = compile_script(&path, &CompileOptions::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
    }
    assert!(compile_error("[Setup]\nArchitecturesAllowed=x86\n").contains("Only x64compatible"));
    assert!(compile_error("[Setup]\nSolidCompression=yes\n").contains("not implemented"));
}
