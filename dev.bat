@echo off
REM Run koharu in dev mode (CPU-only — overrides cuda feature from tauri.windows.conf.json).
call "C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
set LIBCLANG_PATH=C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Tools\Llvm\x64\bin
cd /d F:\Github\koharu
bun x @tauri-apps/cli dev --config "{\"build\":{\"features\":[]}}"
