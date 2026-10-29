; Build-Demo.cmd builds this script using the prebuilt Rust compiler and stub.
#define AppName "IronSetup Demo"
#define AppVersion "1.0.0"

[Setup]
AppId=IronSetup.Demo
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=IronSetup Examples
DefaultDirName={localappdata}\Programs\IronSetup Demo
DefaultGroupName=IronSetup Demo
PrivilegesRequired=lowest
Compression=zip
ArchitecturesAllowed=x64compatible
OutputDir=Output
OutputBaseFilename=IronSetup-Demo-Setup
LicenseFile=payload\LICENSE-demo.txt
InfoBeforeFile=payload\before.txt
InfoAfterFile=payload\after.txt

[Types]
Name: full; Description: "Application and documentation"
Name: minimal; Description: "Application only"
Name: custom; Description: "Choose components"; Flags: iscustom

[Components]
Name: core; Description: "Demo application (required)"; Types: full minimal custom; Flags: fixed
Name: docs; Description: "User guide"; Types: full

[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "payload\windows\IronSetup-Demo.exe"; DestDir: "{app}"; Flags: ignoreversion; Components: core
Source: "payload\readme.txt"; DestDir: "{app}"; Flags: ignoreversion; Components: core
Source: "payload\settings.toml"; DestDir: "{app}\config"; Flags: onlyifdoesntexist; Components: core
Source: "payload\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs; Components: docs

[Dirs]
Name: "{app}\data"

[Icons]
Name: "{group}\IronSetup Demo"; Filename: "{app}\IronSetup-Demo.exe"; WorkingDir: "{app}"; Comment: "Rust installer demonstration"
Name: "{group}\Uninstall IronSetup Demo"; Filename: "{uninstallexe}"
Name: "{userdesktop}\IronSetup Demo"; Filename: "{app}\IronSetup-Demo.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\IronSetupDemo"; ValueType: string; ValueName: "InstallPath"; ValueData: "{app}"; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\IronSetupDemo"; ValueType: string; ValueName: "Version"; ValueData: "{#AppVersion}"; Flags: uninsdeletevalue
