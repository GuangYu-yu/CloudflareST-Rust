@echo off
setlocal enabledelayedexpansion

set "args="
for %%A in (%*) do (
    set "arg=%%A"
    if /I "!arg!"=="/NOLOGO" (
    ) else if /I "!arg:~0,11!"=="/PDBALTPATH:" (
    ) else if /I "!arg:~0,8!"=="/NATVIS:" (
    ) else (
        set "args=!args! %%A"
    )
)
zig cc !args!
