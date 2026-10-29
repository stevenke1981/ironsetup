@echo off
setlocal
cd /d "%~dp0"
"bin\windows-x64\ironsetup.exe" compile "examples\windows-demo.iss" --stub "bin\windows-x64\ironsetup-stub.exe" --output "samples\IronSetup-Demo-Setup.exe"
if errorlevel 1 (
  echo Build failed. Read the diagnostic above.
  pause
  exit /b 1
)
echo.
echo Output: %~dp0samples\IronSetup-Demo-Setup.exe
pause
