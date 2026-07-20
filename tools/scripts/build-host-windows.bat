@echo off
REM Delegates to the canonical script inside the wandr-host submodule.
REM This used to be a full copy and the two had drifted; task 117's libvpx env
REM plumbing must live in one place, so this is now a forwarder.
call "%~dp0..\..\runtime\wandr-host\scripts\build-host-windows.bat" %*
