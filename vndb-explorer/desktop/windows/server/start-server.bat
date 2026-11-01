@echo off
chcp 65001 >nul
rem Сервер в обычном окне (видно журнал). Закройте окно, чтобы остановить.
call "%~dp0config.bat"
echo Откройте в браузере: http://127.0.0.1:%VNSHELF_PORT%/
"%~dp0vndb-web.exe" --data "%~dp0data" serve --addr 127.0.0.1:%VNSHELF_PORT% --web "%~dp0..\client\web"
pause
