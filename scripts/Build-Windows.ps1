param([switch]$GuiSmoke)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'Install Rust stable and Visual Studio C++ Build Tools, then reopen PowerShell.' }
    & cargo test --locked --workspace
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed.' }
    if ($GuiSmoke) {
        & cargo test --locked -p ironsetup-ui --test native_ui_smoke -- --ignored --test-threads=1
        if ($LASTEXITCODE -ne 0) { throw 'Native GUI smoke failed.' }
    }
    & cargo build --locked --release --workspace
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    New-Item -ItemType Directory -Force 'bin\windows-x64','examples\payload\windows','samples' | Out-Null
    foreach ($name in @('ironsetup','ironsetup-stub','ironsetup-studio')) {
        Copy-Item "target\release\$name.exe" "bin\windows-x64\$name.exe" -Force
    }
    Copy-Item 'target\release\ironsetup-demo.exe' 'examples\payload\windows\IronSetup-Demo.exe' -Force
    & '.\bin\windows-x64\ironsetup.exe' compile 'examples\windows-demo.iss' --stub 'bin\windows-x64\ironsetup-stub.exe' --output 'samples\IronSetup-Demo-Setup.exe'
    if ($LASTEXITCODE -ne 0) { throw 'Demo installer compilation failed.' }
    Write-Host 'Build complete. Run Start-IronSetup.cmd or scripts\Test-Windows.ps1.'
} finally { Pop-Location }
