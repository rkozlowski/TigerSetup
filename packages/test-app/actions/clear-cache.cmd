@echo off
if not exist "%~1" mkdir "%~1"
if exist "%~1\cache.txt" del /f /q "%~1\cache.txt"
echo TigerSetupTestAction> "%~1\pre-uninstall.txt"
echo clear-cache	%TIGERSETUP_OPERATION%	%TIGERSETUP_PHASE%>> "%~1\record.txt"
exit /b 0
