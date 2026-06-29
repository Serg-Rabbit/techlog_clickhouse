# Техжурнал ClickHouse

Быстрый анализатор событий `CALL`, `SDBL` и HTTP-пар `VRSREQUEST/VRSRESPONSE` из технологического журнала 1С.

Текущая версия держит легкую схему: одна таблица `techlog.events`. Связи `SDBL -> CALL` и дочерние drilldown-таблицы не рассчитываются, чтобы не замедлять импорт большого файла.

## Docker

Поднять локальное окружение:

```powershell
Copy-Item .env.example .env
# Отредактируйте .env: укажите TECHLOG_INBOX, TECHLOG_UPLOAD_TOKEN и HYPERDX_FRONTEND_URL
docker compose up -d
```

Compose запускает:

- ClickHouse на `http://localhost:8123`;
- базу `techlog`;
- пользователя `techlog` с паролем `techlog`;
- HyperDX на `http://10.0.10.24:8088` по умолчанию.
- `techlog-loader`, который принимает файлы по HTTP и следит за папкой из `TECHLOG_INBOX`.

`TECHLOG_INBOX` в `.env` должен указывать на папку, куда будут попадать `.zip` и `.log` файлы техжурнала. Docker пробрасывает эту папку внутрь контейнера как `/inbox`.
`TECHLOG_UPLOAD_TOKEN` защищает HTTP-загрузку файлов в loader.

Пример для Docker Desktop на Windows:

```env
TECHLOG_INBOX=C:\techlog-inbox
TECHLOG_UPLOAD_TOKEN=change-me
HYPERDX_FRONTEND_URL=http://localhost:8088
```

Пример для Linux-сервера, где Windows-шара уже примонтирована в `/mnt/techlog-inbox`:

```env
TECHLOG_INBOX=/mnt/techlog-inbox
TECHLOG_UPLOAD_TOKEN=change-me
HYPERDX_FRONTEND_URL=http://10.0.10.24:8088
```

HyperDX использует `FRONTEND_URL` для редиректов после авторизации. В `docker-compose.yml` он берется из переменной `HYPERDX_FRONTEND_URL`, а если она не задана, используется `http://10.0.10.24:8088`. Для другого сервера или локального запуска задайте свой адрес:

```powershell
$env:HYPERDX_FRONTEND_URL = "http://localhost:8088"
docker compose up -d
```

На Linux:

```bash
HYPERDX_FRONTEND_URL=http://10.0.10.24:8088 docker compose up -d
```

### Автозагрузка техжурнала

`techlog-loader` принимает `.zip` и `.log` по HTTP на порту `18081` и сохраняет их в `/inbox`. Пример загрузки с Windows:

```powershell
curl.exe -H "Authorization: Bearer change-me" `
  -F "file=@C:\path\26061518.zip" `
  http://10.0.10.24:18081/upload
```

Проверить, что upload endpoint жив:

```powershell
curl.exe http://10.0.10.24:18081/health
```

После сохранения файла `techlog-loader` автоматически обрабатывает файлы в `/inbox`. Эта папка используется только как входящий буфер для `.zip` и `.log`, а архивы распаковываются во временную папку контейнера `/tmp/techlog-loader`, чтобы не гонять большие распакованные файлы через Windows bind mount:

- `.zip`: ждет окончания копирования, распаковывает во временную папку контейнера, импортирует все найденные `.log`, затем удаляет исходный `.zip` и папку распаковки;
- `.log`: ждет окончания копирования, импортирует файл, затем удаляет исходный `.log`.

Русские буквы в имени архива сохраняются в имени временной папки распаковки. Опасные символы пути вроде `/`, `\`, `:` и `..` заменяются безопасным именем.

Перед загрузкой каждого `.log` loader удаляет старые строки с таким же `file_path`, чтобы повторная загрузка того же файла не создавала дубли.

Посмотреть логи загрузчика:

```powershell
docker logs -f techlog-loader
```

Пересобрать loader после изменения Rust-кода:

```powershell
docker compose up -d --build techlog-loader
```

Native-порт ClickHouse `9000` не пробрасывается наружу, чтобы не конфликтовать с уже занятыми портами на сервере. Для импорта и команд этого проекта используется HTTP-порт `8123`. Если native-доступ с хоста всё-таки нужен, добавьте в `docker-compose.yml` свободный внешний порт, например:

```yaml
ports:
  - "8123:8123"
  - "19000:9000"
```

Схема ClickHouse автоматически применяется из `sql/init/001_schema.sql` при первом создании volume `clickhouse-data`. Если volume уже существовал, обновить схему можно командой:

```powershell
cargo run --release -- schema --host localhost --port 8123 --database techlog --user techlog --password techlog
```

В `techlog.events` включен TTL: события хранятся примерно 1 месяц по `event_dt`. ClickHouse удаляет устаревшие данные фоном во время merge, поэтому очистка не происходит мгновенно ровно по расписанию.

Проверить доступность ClickHouse:

```powershell
docker compose ps
docker exec -it techlog-clickhouse clickhouse-client --user techlog --password techlog --query "SELECT 1"
```

Остановить контейнеры:

```powershell
docker compose down
```

Полностью удалить данные ClickHouse и MongoDB:

```powershell
docker compose down -v
```

## Команды

Создать или обновить схему ClickHouse:

```powershell
cargo run --release -- schema --host localhost --port 8123 --database techlog --user techlog --password techlog
```

Просканировать папку рекурсивно:

```powershell
cargo run --release -- scan --path "Файлы техжурнала"
cargo run --release -- scan --path "Файлы техжурнала" --count-lines
```

Импортировать журналы:

```powershell
cargo run --release -- import --path "Файлы техжурнала" --host localhost --port 8123 --database techlog --user techlog --password techlog
```

Импорт делает только создание схемы, опциональную очистку `techlog.events`, потоковое чтение `.log` файлов и батчевую вставку `CALL`, `SDBL` и свернутых HTTP-пар `VRSREQUEST/VRSRESPONSE` как `VRSREQRESP` в ClickHouse.

Для `VRSREQRESP` поле `place` заполняется как `Method URI` без query-параметров и с заменой GUID на `<GUID>`, `first_context_line` содержит HTTP-статус ответа, а полный исходный `Method URI` сохраняется в `stack_text`.

Импорт по умолчанию оптимизирован по скорости:

- вставка использует `TabSeparated`;
- размер батча до `100000` строк или `64 MB`;
- `raw_record` остается пустым, чтобы не дублировать большие фрагменты журнала.

Используйте `--store-raw-record` только когда полный исходный текст записи действительно нужен в ClickHouse. Этот режим может заметно замедлить импорт и увеличить размер таблицы.

Для диагностики доступен запасной формат вставки JSON:

```powershell
cargo run --release -- import --path "Файлы техжурнала" --insert-format json
```

Используйте `--truncate` только если нужно очистить `techlog.events` перед загрузкой:

```powershell
cargo run --release -- import --path "Файлы техжурнала" --truncate
```

## Файлы

- `src/main.rs` содержит потоковый парсер, HTTP-клиент ClickHouse и CLI.
- `sql/init/001_schema.sql` создает `techlog.events` и удаляет старые drilldown-объекты.
- `samples/26061715.log` содержит небольшой пример для интеграционной проверки.

