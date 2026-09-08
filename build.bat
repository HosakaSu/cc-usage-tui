@echo off
REM Build cc-usage-tui. rusqlite's bundled SQLite needs a C compiler (MSVC cl.exe),
REM so locate Visual Studio Build Tools via vswhere and enter its x64 environment.
setlocal

REM Capture ProgramFiles(x86) once, OUTSIDE any parenthesised block: its ")" would
REM otherwise be parsed as the end of an "if ( ... )" block.
set "PF86=%ProgramFiles(x86)%"
set "VSWHERE=%PF86%\Microsoft Visual Studio\Installer\vswhere.exe"
set "VCVARS="

if exist "%VSWHERE%" for /f "usebackq tokens=*" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2^>nul`) do if exist "%%i\VC\Auxiliary\Build\vcvars64.bat" set "VCVARS=%%i\VC\Auxiliary\Build\vcvars64.bat"

if not defined VCVARS if exist "%PF86%\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" set "VCVARS=%PF86%\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"

if defined VCVARS echo [build] using MSVC env: %VCVARS%
if defined VCVARS call "%VCVARS%" >nul 2>&1
if not defined VCVARS echo [build] WARNING: vcvars64.bat not found; relying on existing environment.

cargo build --release %*
if errorlevel 1 echo [build] FAILED & exit /b 1

echo.
echo [build] OK -^> target\release\cc-usage-tui.exe
endlocal
