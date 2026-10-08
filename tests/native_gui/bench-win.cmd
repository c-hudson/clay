@echo off
rem Windows half of the native GUI's Phase-0 bench (tests/native_gui/bench.sh is Linux).
rem Run in the interactive desktop session (schtasks /it), one GUI at a time so a
rem screenshot of the VM's screen shows only that window:
rem   bench-win.cmd webview "CJK,Emoji"
rem   bench-win.cmd native  "CJK,Emoji"
rem   bench-win.cmd stop
rem Expects %USERPROFILE%\clay-native built with --features webview-gui,native-gui.
setlocal
set ROOT=%USERPROFILE%\clay-native
set BIN=%ROOT%\target\debug
set WORK=%USERPROFILE%\nbench

taskkill /f /im clay.exe >nul 2>&1
taskkill /f /im clay-test-server.exe >nul 2>&1
if "%~1"=="stop" goto :eof

call :home "%WORK%\home-webview" 19101 19201
call :home "%WORK%\home-native" 19102 19202

set CLAY_CORPUS_SECTIONS=%~2
start "" /b "%BIN%\clay-test-server.exe" --ports=19101-19102 --scenarios=corpus,corpus
timeout /t 1 /nobreak >nul

if "%~1"=="webview" (
    set USERPROFILE=%WORK%\home-webview
    start "" "%BIN%\clay.exe" --gui
    goto :eof
)
set USERPROFILE=%WORK%\home-native
set CLAY_WS_PASSWORD=bench
start "" /b "%BIN%\clay.exe" --local-server --port=19202
timeout /t 3 /nobreak >nul
set CLAY_PASSWORD=bench
start "" "%BIN%\clay.exe" --native-gui=127.0.0.1:19202
goto :eof

:home
rem Clay keeps its data in %USERPROFILE%\clay on Windows.
if exist "%~1" rmdir /s /q "%~1"
mkdir "%~1\clay"
(
echo [global]
echo http_enabled=true
echo http_port=%3
echo more_mode=false
echo.
echo [world:Corpus]
echo world_type=mud
echo hostname=127.0.0.1
echo port=%2
echo use_ssl=false
echo.
echo [action:bench-connect]
echo command=/connect Corpus
echo startup=true
) > "%~1\clay\settings.dat"
goto :eof
