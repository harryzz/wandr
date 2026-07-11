@echo off
REM Launch an installed wandr app on the Windows desktop host.
REM
REM Usage:  run-app-windows.bat [app-id]        (default: wandr.audio.player)
REM
REM Apps must already be installed into WANDR_APPS_ROOT
REM   (<host>.exe --install <pkg-dir>). Build the host first with
REM   build-host-windows.bat.
REM
REM The window auto-fits: a phone shape (520x1040) with its height capped to 90%%
REM of the primary screen's work area, so it never runs off a small laptop screen
REM (the Signal QR/link screen). Set WANDR_DESKTOP_SIZE to override.
REM
REM Env overrides: WANDR_HOST, WANDR_APPS_ROOT, FFMPEG_DIR, WANDR_DESKTOP_SIZE
setlocal
set "APP=%~1"
if "%APP%"=="" set "APP=wandr.audio.player"

if "%WANDR_HOST%"==""      set "WANDR_HOST=%~dp0..\..\runtime\wandr-host\target\release\wasm-android-host.exe"
if "%WANDR_APPS_ROOT%"=="" set "WANDR_APPS_ROOT=%USERPROFILE%\wandr-apps"
if "%FFMPEG_DIR%"==""      set "FFMPEG_DIR=C:\wandr-win\ff\ffmpeg-n8.1-latest-win64-gpl-shared-8.1"
if "%RUST_LOG%"==""        set "RUST_LOG=info"

REM ffmpeg DLLs (avcodec-62 etc.) are load-time deps — put its bin on PATH.
set "PATH=%FFMPEG_DIR%\bin;%PATH%"

REM Auto-fit the window height to the screen (phone aspect preserved).
if "%WANDR_DESKTOP_SIZE%"=="" (
  for /f "usebackq delims=" %%S in (`powershell -NoProfile -Command "Add-Type -AssemblyName System.Windows.Forms; $h=[System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea.Height; $mh=[math]::Floor($h*0.9); if(1040 -gt $mh){$H=$mh;$W=[math]::Floor(520*$mh/1040)}else{$H=1040;$W=520}; \"$W`x$H\""`) do set "WANDR_DESKTOP_SIZE=%%S"
)
if "%WANDR_DESKTOP_SIZE%"=="" set "WANDR_DESKTOP_SIZE=460x920"

if not exist "%WANDR_HOST%" (
  echo host not built: %WANDR_HOST% >&2
  echo run tools\scripts\build-host-windows.bat first, or set WANDR_HOST. >&2
  exit /b 1
)

echo launching %APP%  (apps: %WANDR_APPS_ROOT%, size: %WANDR_DESKTOP_SIZE%)
"%WANDR_HOST%" --app %APP%
endlocal
