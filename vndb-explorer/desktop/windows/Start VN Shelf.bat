@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"
call "%~dp0server\config.bat"

call :running && goto open
echo Запускаю сервер VN Shelf (порт %VNSHELF_PORT%)...
start "VN Shelf server" /min "%~dp0server\vndb-web.exe" --data "%~dp0server\data" serve --addr 127.0.0.1:%VNSHELF_PORT% --web "%~dp0client\web"

set /a tries=0
:wait
call :running && goto open
set /a tries+=1
if %tries% geq 60 (
  echo Сервер не запустился. Посмотрите окно "VN Shelf server" или запустите server\start-server.bat.
  pause
  exit /b 1
)
ping -n 2 127.0.0.1 >nul
goto wait

:open
call "%~dp0client\open-client.bat"
exit /b 0

:running
powershell -NoProfile -Command "try { $null = Invoke-WebRequest -UseBasicParsing -TimeoutSec 2 http://127.0.0.1:%VNSHELF_PORT%/api/status; exit 0 } catch { exit 1 }" >nul 2>&1
exit /b %errorlevel%
