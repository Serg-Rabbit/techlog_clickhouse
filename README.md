# Техжурнал ClickHouse

Быстрый анализатор событий `CALL` и `SDBL` из технологического журнала 1С.

Текущая версия держит легкую схему: одна таблица `techlog.events`. Связи `SDBL -> CALL` и дочерние drilldown-таблицы не рассчитываются, чтобы не замедлять импорт большого файла.

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

