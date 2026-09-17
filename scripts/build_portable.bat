@echo off
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "build_portable.ps1"
exit /b %ERRORLEVEL%
