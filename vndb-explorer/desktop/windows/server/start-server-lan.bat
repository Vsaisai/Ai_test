@echo off
chcp 65001 >nul
rem Сервер, доступный с телефона в той же сети Wi-Fi.
rem Windows спросит разрешение в брандмауэре — разрешите для частных сетей.
rem Пароля нет: запускайте только в домашней сети.
call "%~dp0config.bat"
echo Откройте на телефоне: http://IP-этого-компьютера:%VNSHELF_PORT%/
echo Адреса этого компьютера:
ipconfig | findstr /i "IPv4"
"%~dp0vndb-web.exe" --data "%~dp0data" serve --addr 0.0.0.0:%VNSHELF_PORT% --web "%~dp0..\client\web"
pause
