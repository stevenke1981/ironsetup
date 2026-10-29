; Advanced example: launch the demo after committing installation.
; nowait reports launch success; it cannot report the program's final exit code.
#include "windows-demo.iss"

[Run]
Filename: "{app}\IronSetup-Demo.exe"; WorkingDir: "{app}"; Flags: nowait skipifsilent
