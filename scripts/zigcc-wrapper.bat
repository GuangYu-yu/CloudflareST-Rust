@echo off
setlocal DisableDelayedExpansion
set "args="
for %%A in (%*) do call :filter "%%~A"
zig cc %args%
exit /b %ERRORLEVEL%

:filter
setlocal EnableDelayedExpansion
set "arg=%~1"
if /I "%arg%"=="/NOLOGO" exit /b
if /I "%arg:~0,11%"=="/PDBALTPATH:" exit /b
if /I "%arg:~0,8%"=="/NATVIS:" exit /b
endlocal & set "args=%args% %~1"
exit /b
