# Работа в GitLab

Репозиторий: https://gitlab.com/Asgefka/Ai_test (приватный).
Импорт из Vsaisai/Ai_test сохраняет Git-историю, ветки и теги.
Основная ветка main старее beta/mobile. Ветки не объединялись автоматически.

## CI/CD

Конфигурация добавлена в main, beta и mobile. Pipeline запускается на push,
теги и merge request, а также вручную через Build → Pipelines → New pipeline.
Сначала выполняются существующие Rust-тесты. Затем собираются Linux ZIP
(если в ветке есть desktop/package.sh) и Android APK для трёх ABI.
Артефакты доступны в заданиях linux-package и android-apk, хранятся 30 дней.
Все команды Cargo для тестов и ПК используют Cargo.lock (--locked).
Android использует существующий mobile/build-apk.sh, JDK 17, SDK 35, NDK 27.2.

Нужен доступный Linux Docker runner с выходом к crates.io, Docker Hub,
dl.google.com, Maven и Gradle. GitLab.com может потребовать верификацию
аккаунта или доступные compute minutes перед выполнением заданий.

На beta Windows, Linux и Android собираются автоматически после тестов.
Windows использует раннер GitLab.com с тегом `saas-windows-medium-amd64`.

Чтобы включить ручное задание `macos-package`, подключите runner с тегом `saas-macos-medium-m1` и задайте переменную проекта `VNSHELF_MACOS_RUNNER=enabled`. Создайте pipeline ветки `beta` и запустите задание.

## Подпись APK

Без переменных используется уже имеющийся debug.keystore — только для тестирования.
Для прежней release-подписи добавьте в Settings → CI/CD → Variables:
ANDROID_KEYSTORE_BASE64, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS,
ANDROID_KEY_PASSWORD. Секреты — masked; protected только если сборочная ветка
или тег тоже protected. Значения GitHub Secrets автоматически не переносятся.
Для обновления ранее установленного приложения нужен тот же ключ подписи.

## Релизы

Сборки веток доступны по ссылкам в [README](README.md#скачать). CI хранит их как артефакты заданий.
Для длительного хранения используйте тег: `pc-v<версия>` на коммите ветки `beta` или
`mobile-v<версия>` на коммите `mobile`. Номер должен совпадать с файлом VERSION соответствующей платформы.
CI кладёт Windows и Linux ZIP или подписанный APK в Generic Package Registry этого проекта
и прикрепляет файлы к GitLab Release. Для Android настройте прежний ключ подписи до создания тега.
Приватные сборки скачиваются после входа в GitLab через браузер. Токен в приложение не встраивается.
