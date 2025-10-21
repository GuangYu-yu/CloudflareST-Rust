@echo off
setlocal DisableDelayedExpansion

rem Detect Windows SDK and MSVC paths
if defined VSINSTALLDIR (
    set "MSVC_LIB=%VSINSTALLDIR%VC\Tools\MSVC\*\lib"
)
if defined WindowsSdkDir (
    set "SDK_UM=%WindowsSdkDir%Lib\*\um"
    set "SDK_UCRT=%WindowsSdkDir%Lib\*\ucrt"
)

rem Choose architecture
if "%1"=="--target=aarch64-pc-windows-msvc" (
    set "LIBPATHS=%MSVC_LIB%\arm64;%SDK_UM%\arm64;%SDK_UCRT%\arm64"
) else (
    set "LIBPATHS=%MSVC_LIB%\x64;%SDK_UM%\x64;%SDK_UCRT%\x64"
)

set "args="
for %%A in (%*) do call :filter "%%~A"
zig cc %args% --libpath %LIBPATHS%
exit /b %ERRORLEVEL%

:filter
setlocal EnableDelayedExpansion
set "arg=%~1"
if /I "%arg%"=="/NOLOGO" exit /b
if /I "%arg:~0,11%"=="/PDBALTPATH:" exit /b
if /I "%arg:~0,8%"=="/NATVIS:" exit /b
endlocal & set "args=%args% %~1"
exit /b
