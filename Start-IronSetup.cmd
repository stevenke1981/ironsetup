@echo off
setlocal
cd /d "%~dp0"
if not exist "bin\windows-x64\ironsetup-studio.exe" (
  echo The Windows portable binaries are missing. Run scripts\Build-Windows.ps1 first.
  pause
  exit /b 1
)
start "" "bin\windows-x64\ironsetup-studio.exe" "%~dp0examples\windows-demo.iss"
