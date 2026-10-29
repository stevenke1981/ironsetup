; Cross-platform filesystem-only example. This script does not create Windows actions.
#define AppName "IronSetup Hello"
#define AppVersion "1.0.0"

[Setup]
AppId=IronSetup.Hello
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=IronSetup Examples
DefaultDirName={userpf}\IronSetupHello
DefaultGroupName={#AppName}
PrivilegesRequired=lowest
Compression=zip
OutputDir=Output
OutputBaseFilename=IronSetup-Hello
LicenseFile=payload\LICENSE-demo.txt

[Types]
Name: full; Description: "Application and documentation"
Name: minimal; Description: "Application only"
Name: custom; Description: "Custom selection"; Flags: iscustom

[Components]
Name: core; Description: "Application files"; Types: full minimal custom; Flags: fixed
Name: docs; Description: "Documentation"; Types: full

[Files]
Source: "payload\readme.txt"; DestDir: "{app}"; Flags: ignoreversion; Components: core
Source: "payload\settings.toml"; DestDir: "{app}\config"; Flags: onlyifdoesntexist; Components: core
Source: "payload\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs; Components: docs

[Dirs]
Name: "{app}\data"
