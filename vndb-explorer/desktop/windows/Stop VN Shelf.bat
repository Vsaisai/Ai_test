@echo off
chcp 65001 >nul
taskkill /im vndb-web.exe /f >nul 2>&1 && echo Сервер VN Shelf остановлен. || echo Сервер VN Shelf не запущен.
timeout /t 2 >nul
