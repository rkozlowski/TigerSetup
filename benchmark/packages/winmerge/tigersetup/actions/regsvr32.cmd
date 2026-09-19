@echo off
rem Registers or unregisters WinMerge's COM shell extension with the regsvr32
rem the command shell resolves (the one on PATH, System32 for the 64-bit
rem engine that runs this), rather than a path written into the package.
rem Arguments are regsvr32's own: /s "<dll>" to register, /u /s "<dll>" to
rem unregister. regsvr32's exit code is this script's.
regsvr32.exe %*
exit /b %ERRORLEVEL%
