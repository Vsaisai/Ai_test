@echo off
rem Открывает интерфейс VN Shelf отдельным окном (как приложение) в Edge или Chrome,
rem а если их нет — в браузере по умолчанию. Сервер должен быть запущен.
call "%~dp0..\server\config.bat"
set "URL=http://127.0.0.1:%VNSHELF_PORT%/"
for %%B in ("%ProgramFiles(x86)%\Microsoft\Edge\Application\msedge.exe" "%ProgramFiles%\Microsoft\Edge\Application\msedge.exe" "%ProgramFiles%\Google\Chrome\Application\chrome.exe" "%ProgramFiles(x86)%\Google\Chrome\Application\chrome.exe" "%LocalAppData%\Google\Chrome\Application\chrome.exe") do (
  if exist %%B (
    start "" %%B --app=%URL%
    exit /b 0
  )
)
start "" "%URL%"
