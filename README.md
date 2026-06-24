# Техжурнал ClickHouse

Быстрый анализатор событий `CALL` и `SDBL` из технологического журнала 1С.

Текущая версия держит легкую схему: одна таблица `techlog.events`. Связи `SDBL -> CALL` и дочерние drilldown-таблицы не рассчитываются, чтобы не замедлять импорт большого файла.

## Docker

Поднять локальное окружение:

```powershell
Copy-Item .env.example .env
# Отредактируйте .env: укажите TECHLOG_INBOX и HYPERDX_FRONTEND_URL
docker compose up -d
```

Compose запускает:

- ClickHouse на `http://localhost:8123`;
- базу `techlog`;
- пользователя `techlog` с паролем `techlog`;
- HyperDX на `http://10.0.10.24:8088` по умолчанию.
- `techlog-loader`, который следит за папкой из `TECHLOG_INBOX`.

`TECHLOG_INBOX` в `.env` должен указывать на папку, куда будут попадать `.zip` и `.log` файлы техжурнала. Docker пробрасывает эту папку внутрь контейнера как `/inbox`.

Пример для Docker Desktop на Windows:

```env
TECHLOG_INBOX=C:\techlog-inbox
HYPERDX_FRONTEND_URL=http://localhost:8088
```

Пример для Linux-сервера, где Windows-шара уже примонтирована в `/mnt/techlog-inbox`:

```env
TECHLOG_INBOX=/mnt/techlog-inbox
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

`techlog-loader` автоматически обрабатывает файлы в `/inbox`:

- `.zip`: ждет окончания копирования, распаковывает в `/inbox/_extracted/<имя архива>/`, импортирует все найденные `.log`, затем удаляет исходный `.zip`;
- `.log`: ждет окончания копирования, импортирует файл, затем удаляет исходный `.log`.

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

Импорт делает только создание схемы, опциональную очистку `techlog.events`, потоковое чтение `.log` файлов и батчевую вставку `CALL/SDBL` в ClickHouse.

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

