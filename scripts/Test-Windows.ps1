$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$compiler = Join-Path $projectRoot 'bin\windows-x64\ironsetup.exe'
$setup = Join-Path $projectRoot 'samples\IronSetup-Demo-Setup.exe'
if (-not (Test-Path $compiler) -or -not (Test-Path $setup)) { throw 'Portable binaries/sample missing. Run scripts\Build-Windows.ps1.' }
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('IronSetupAcceptance-' + [guid]::NewGuid().ToString('N'))
$target = Join-Path $fixtureRoot 'Installed Demo'
$log = Join-Path $fixtureRoot 'setup.log'
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
try {
    $process = Start-Process -FilePath $setup -ArgumentList @('/VERYSILENT',('/DIR="'+$target+'"'),'/TASKS=','/NORESTART',('/LOG="'+$log+'"')) -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer failed: $($process.ExitCode). See $log" }
    if (-not (Test-Path (Join-Path $target 'IronSetup-Demo.exe'))) { throw 'Installed application is missing.' }
    if (-not (Test-Path (Join-Path $target 'unins000.exe'))) { throw 'Uninstaller is missing.' }
    & $compiler verify --dir $target
    if ($LASTEXITCODE -ne 0) { throw 'Installed file verification failed.' }
    $userFile = Join-Path $target 'my-private-notes.txt'
    [IO.File]::WriteAllText($userFile,'Preserve this user file.')
    [IO.File]::AppendAllText((Join-Path $target 'readme.txt'),'User edit.')
    & $compiler verify --dir $target
    if ($LASTEXITCODE -ne 3) { throw 'Modified file was not reported with exit 3.' }
    & $compiler uninstall --dir $target
    if ($LASTEXITCODE -ne 0) { throw 'Uninstall failed.' }
    if (-not (Test-Path $userFile) -or -not (Test-Path (Join-Path $target 'readme.txt'))) { throw 'User data was not preserved.' }
    if (Test-Path (Join-Path $target 'IronSetup-Demo.exe')) { throw 'Unmodified installed application was not removed.' }
    Write-Host 'PASS: Windows SFX install, verify, modification detection, uninstall and user-file preservation.'
    Write-Host "Evidence and retained fixtures: $fixtureRoot"
} catch {
    Write-Error "Acceptance failed. Retained evidence: $fixtureRoot. $_"
    exit 1
}
