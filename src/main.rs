use std::env;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Instant;

const TARGET_EVENTS: [&str; 2] = ["CALL", "SDBL"];
const DEFAULT_BATCH_ROWS: usize = 100_000;
const DEFAULT_BATCH_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
struct Config {
    command: Command,
    path: Option<PathBuf>,
    host: String,
    port: u16,
    database: String,
    user: String,
    password: String,
    batch_rows: usize,
    batch_bytes: usize,
    insert_format: InsertFormat,
    store_raw_record: bool,
    truncate: bool,
    count_lines: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Schema,
    Import,
    Scan,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertFormat {
    Tsv,
    Json,
}

#[derive(Debug, Clone)]
struct FileDate {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    text_prefix: String,
}

#[derive(Debug, Clone)]
struct Header {
    time: String,
    duration_us: u64,
    event_name: String,
    fields_start: usize,
}

#[derive(Debug, Clone)]
struct ParsedEvent {
    event_key: String,
    line_no: u64,
    usr: String,
    session_id: String,
    event_name: String,
    event_time_text: String,
    event_dt: String,
    duration_us: u64,
    cpu_time_us: u64,
    func: String,
    form: String,
    form_item: String,
    iname: String,
    mname: String,
    method: String,
    module: String,
    place: String,
    first_context_line: String,
    query_text: String,
    stack_text: String,
    file_path: String,
    raw_record: String,
}

#[derive(Debug, Default)]
struct ParsedFields {
    usr: String,
    session_id: String,
    context: String,
    cpu_time: String,
    func: String,
    form: String,
    form_item: String,
    iname: String,
    mname: String,
    method: String,
    module: String,
    sdbl: String,
    table_name: String,
}

#[derive(Debug, Default)]
struct ImportStats {
    files: u64,
    bytes: u64,
    lines: u64,
    records: u64,
    inserted: u64,
    skipped: u64,
    errors: u64,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cfg = parse_args(env::args().skip(1).collect())?;
    match cfg.command {
        Command::Help => {
            print_help();
            Ok(())
        }
        Command::Schema => run_schema(&cfg),
        Command::Scan => run_scan(&cfg),
        Command::Import => run_import(&cfg),
    }
}

fn parse_args(args: Vec<String>) -> Result<Config, String> {
    let mut cfg = Config {
        command: Command::Help,
        path: None,
        host: "localhost".to_string(),
        port: 8123,
        database: "techlog".to_string(),
        user: "techlog".to_string(),
        password: "techlog".to_string(),
        batch_rows: DEFAULT_BATCH_ROWS,
        batch_bytes: DEFAULT_BATCH_BYTES,
        insert_format: InsertFormat::Tsv,
        store_raw_record: false,
        truncate: false,
        count_lines: false,
    };

    if args.is_empty() {
        return Ok(cfg);
    }

    cfg.command = match args[0].as_str() {
        "schema" => Command::Schema,
        "import" => Command::Import,
        "scan" => Command::Scan,
        "help" | "-h" | "--help" => Command::Help,
        other => return Err(format!("unknown command: {other}")),
    };

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--path" => {
                i += 1;
                cfg.path = Some(PathBuf::from(value_at(&args, i, "--path")?));
            }
            "--host" => {
                i += 1;
                cfg.host = value_at(&args, i, "--host")?.to_string();
            }
            "--port" => {
                i += 1;
                cfg.port = value_at(&args, i, "--port")?
                    .parse()
                    .map_err(|_| "--port must be a number".to_string())?;
            }
            "--database" => {
                i += 1;
                cfg.database = value_at(&args, i, "--database")?.to_string();
            }
            "--user" => {
                i += 1;
                cfg.user = value_at(&args, i, "--user")?.to_string();
            }
            "--password" => {
                i += 1;
                cfg.password = value_at(&args, i, "--password")?.to_string();
            }
            "--batch-rows" => {
                i += 1;
                cfg.batch_rows = value_at(&args, i, "--batch-rows")?
                    .parse()
                    .map_err(|_| "--batch-rows must be a number".to_string())?;
            }
            "--batch-bytes" => {
                i += 1;
                cfg.batch_bytes = value_at(&args, i, "--batch-bytes")?
                    .parse()
                    .map_err(|_| "--batch-bytes must be a number".to_string())?;
            }
            "--insert-format" => {
                i += 1;
                cfg.insert_format = match value_at(&args, i, "--insert-format")? {
                    "tsv" | "TabSeparated" | "tabseparated" => InsertFormat::Tsv,
                    "json" | "JSONEachRow" | "jsoneachrow" => InsertFormat::Json,
                    other => return Err(format!("unsupported --insert-format: {other}")),
                };
            }
            "--store-raw-record" => cfg.store_raw_record = true,
            "--truncate" => cfg.truncate = true,
            "--count-lines" => cfg.count_lines = true,
            "-h" | "--help" => cfg.command = Command::Help,
            other => return Err(format!("unknown option: {other}")),
        }
        i += 1;
    }

    if matches!(cfg.command, Command::Import | Command::Scan) && cfg.path.is_none() {
        return Err("--path is required".to_string());
    }

    Ok(cfg)
}

fn value_at<'a>(args: &'a [String], index: usize, option: &str) -> Result<&'a str, String> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("{option} requires a value"))
}

fn print_help() {
    println!(
        "Использование:\n  cargo run --release -- schema [--host localhost --port 8123 --database techlog --user techlog --password techlog]\n  cargo run --release -- import --path \"Файлы техжурнала\" [--truncate] [--insert-format tsv|json] [--store-raw-record]\n  cargo run --release -- scan --path \"Файлы техжурнала\" [--count-lines]"
    );
}

fn run_schema(cfg: &Config) -> Result<(), String> {
    let sql = include_str!("../sql/init/001_schema.sql").replace("techlog", &cfg.database);
    for statement in split_sql_statements(&sql) {
        clickhouse_query(cfg, &statement)?;
    }
    println!("schema is ready: {}", cfg.database);
    Ok(())
}

fn run_scan(cfg: &Config) -> Result<(), String> {
    let root = cfg.path.as_ref().expect("validated path");
    let files = collect_log_files(root)?;
    let mut total_bytes = 0u64;
    let mut total_lines = 0u64;
    for file in &files {
        total_bytes += fs::metadata(file)
            .map_err(|e| format!("{}: {e}", file.display()))?
            .len();
        if cfg.count_lines {
            total_lines += count_lines(file)?;
        }
    }
    println!("files: {}", files.len());
    println!("bytes: {}", total_bytes);
    println!("megabytes: {:.1}", total_bytes as f64 / 1024.0 / 1024.0);
    if cfg.count_lines {
        println!("lines: {}", total_lines);
    }
    Ok(())
}

fn run_import(cfg: &Config) -> Result<(), String> {
    run_schema(cfg)?;
    if cfg.truncate {
        clickhouse_query(
            cfg,
            &format!("TRUNCATE TABLE {}.events", ident(&cfg.database)),
        )?;
    }

    let root = cfg.path.as_ref().expect("validated path");
    let files = collect_log_files(root)?;
    if files.is_empty() {
        return Err(format!("no .log files found in {}", root.display()));
    }

    let started = Instant::now();
    let mut stats = ImportStats::default();
    let mut batch = InsertBatch::new(cfg);

    for file in files {
        stats.files += 1;
        stats.bytes += fs::metadata(&file)
            .map_err(|e| format!("{}: {e}", file.display()))?
            .len();
        import_file(&file, cfg, &mut batch, &mut stats)?;
    }
    batch.flush(cfg, &mut stats)?;

    let elapsed = started.elapsed().as_secs_f64();
    println!("files: {}", stats.files);
    println!("bytes: {}", stats.bytes);
    println!("lines: {}", stats.lines);
    println!("records: {}", stats.records);
    println!("inserted: {}", stats.inserted);
    println!("skipped: {}", stats.skipped);
    println!("errors: {}", stats.errors);
    println!("seconds: {:.3}", elapsed);
    if elapsed > 0.0 {
        println!(
            "inserted_per_second: {:.0}",
            stats.inserted as f64 / elapsed
        );
    }
    Ok(())
}

fn import_file(
    path: &Path,
    cfg: &Config,
    batch: &mut InsertBatch,
    stats: &mut ImportStats,
) -> Result<(), String> {
    let date = file_date(path);
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let file_path = path.to_string_lossy().to_string();
    let mut buf = Vec::with_capacity(8192);
    let mut current_record = String::with_capacity(8192);
    let mut current_start_line = 0u64;
    let mut line_no = 0u64;

    loop {
        buf.clear();
        let read = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        line_no += 1;
        stats.lines += 1;
        trim_newline_bytes(&mut buf);

        if let Some(event_name) = event_name_at_record_start_bytes(&buf) {
            if !current_record.is_empty() {
                process_record(
                    &current_record,
                    &file_path,
                    current_start_line,
                    date.as_ref(),
                    cfg,
                    batch,
                    stats,
                )?;
            }
            current_record.clear();
            current_start_line = 0;
            if is_target_event(event_name) {
                current_start_line = line_no;
                append_record_line(&mut current_record, &buf, line_no == 1);
            }
        } else if current_start_line != 0 {
            append_record_line(&mut current_record, &buf, false);
        }
    }

    if !current_record.is_empty() {
        process_record(
            &current_record,
            &file_path,
            current_start_line,
            date.as_ref(),
            cfg,
            batch,
            stats,
        )?;
    }

    Ok(())
}

fn process_record(
    record: &str,
    file_path: &str,
    line_no: u64,
    date: Option<&FileDate>,
    cfg: &Config,
    batch: &mut InsertBatch,
    stats: &mut ImportStats,
) -> Result<(), String> {
    stats.records += 1;
    match parse_record(record, file_path, line_no, date, cfg.store_raw_record) {
        Some(event) => batch.push(&event, cfg, stats),
        None => {
            stats.skipped += 1;
            Ok(())
        }
    }
}

fn parse_record(
    record: &str,
    file_path: &str,
    line_no: u64,
    date: Option<&FileDate>,
    store_raw_record: bool,
) -> Option<ParsedEvent> {
    let header = parse_header(record)?;
    if !TARGET_EVENTS.contains(&header.event_name.as_str()) {
        return None;
    }

    let fields = extract_fields(record, header.fields_start, &header.event_name);
    let (first_context_line, last_context_line) = context_bounds(&fields.context);

    let place = match header.event_name.as_str() {
        "CALL" => call_place(
            &last_context_line,
            &fields.module,
            &fields.method,
            &fields.func,
            &fields.form,
            &fields.form_item,
            &fields.iname,
            &fields.mname,
        ),
        "SDBL" => sdbl_place(&last_context_line, &fields.func, &fields.table_name),
        _ => String::new(),
    };

    let cpu_time_us = parse_number_u64(&fields.cpu_time);
    let event_dt = event_datetime(date, &header.time);
    let event_time_text = match date {
        Some(d) => format!("{}{}", d.text_prefix, header.time),
        None => header.time.clone(),
    };

    Some(ParsedEvent {
        event_key: format!("{file_path}|{line_no}"),
        line_no,
        usr: fields.usr,
        session_id: fields.session_id,
        event_name: header.event_name,
        event_time_text,
        event_dt,
        duration_us: header.duration_us,
        cpu_time_us,
        func: fields.func,
        form: fields.form,
        form_item: fields.form_item,
        iname: fields.iname,
        mname: fields.mname,
        method: fields.method,
        module: fields.module,
        place: truncate_chars(&place, 1000),
        first_context_line: truncate_chars(&first_context_line, 1000),
        query_text: fields.sdbl,
        stack_text: fields.context,
        file_path: file_path.to_string(),
        raw_record: if store_raw_record {
            record.to_string()
        } else {
            String::new()
        },
    })
}

#[cfg(test)]
fn event_name_at_record_start(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let event = event_name_at_record_start_bytes(bytes)?;
    std::str::from_utf8(event).ok().map(str::to_string)
}

fn event_name_at_record_start_bytes(bytes: &[u8]) -> Option<&[u8]> {
    let offset = if bytes.starts_with(b"\xef\xbb\xbf") {
        3
    } else {
        0
    };
    if bytes.len() < offset + 7
        || bytes.get(offset + 2) != Some(&b':')
        || bytes.get(offset + 5) != Some(&b'.')
    {
        return None;
    }
    let first = find_byte(bytes, offset, b',')?;
    let second = find_byte(bytes, first + 1, b',')?;
    Some(&bytes[first + 1..second])
}

fn is_target_event(event_name: &[u8]) -> bool {
    event_name == b"CALL" || event_name == b"SDBL"
}

fn append_record_line(record: &mut String, bytes: &[u8], strip_bom: bool) {
    if !record.is_empty() {
        record.push('\n');
    }
    let bytes = if strip_bom && bytes.starts_with(b"\xef\xbb\xbf") {
        &bytes[3..]
    } else {
        bytes
    };
    let line = String::from_utf8_lossy(bytes);
    record.push_str(&line);
}

fn find_byte(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    bytes
        .get(start..)?
        .iter()
        .position(|b| *b == needle)
        .map(|rel| start + rel)
}

fn parse_header(record: &str) -> Option<Header> {
    let record = strip_utf8_bom(record);
    let first = record.find(',')?;
    let second = record[first + 1..].find(',')? + first + 1;
    let fields_start = record[second + 1..]
        .find(',')
        .map(|rel| second + 1 + rel + 1)
        .unwrap_or(record.len());
    let dash = record[..first].find('-')?;
    let time = record[..dash].to_string();
    let duration_us = record[dash + 1..first].parse().ok()?;
    let event_name = record[first + 1..second].to_string();
    Some(Header {
        time,
        duration_us,
        event_name,
        fields_start,
    })
}

fn extract_fields(record: &str, fields_start: usize, event_name: &str) -> ParsedFields {
    let mut fields = ParsedFields::default();
    if fields_start >= record.len() {
        return fields;
    }

    let mut pos = fields_start;
    while pos < record.len() {
        while record.as_bytes().get(pos) == Some(&b',') {
            pos += 1;
        }
        if pos >= record.len() {
            break;
        }

        let bytes = record.as_bytes();
        let next_comma = find_byte(bytes, pos, b',');
        let Some(eq) = find_byte(bytes, pos, b'=') else {
            break;
        };
        if next_comma.is_some_and(|comma| comma < eq) {
            pos = next_comma.unwrap() + 1;
            continue;
        }
        let name = &record[pos..eq];
        pos = if is_relevant_field(event_name, name) {
            let (value, next_pos) = read_field_value(record, eq + 1);
            set_field(&mut fields, event_name, name, value);
            next_pos
        } else {
            skip_field_value(record, eq + 1)
        };
        if record.as_bytes().get(pos) == Some(&b',') {
            pos += 1;
        }
    }

    fields
}

fn is_relevant_field(event_name: &str, name: &str) -> bool {
    matches!(name, "Usr" | "SessionID" | "Context" | "CpuTime" | "Func")
        || (event_name == "CALL"
            && matches!(
                name,
                "IName" | "MName" | "Module" | "Method" | "Form" | "FormItem"
            ))
        || (event_name == "SDBL" && matches!(name, "Sdbl" | "tableName"))
}

fn set_field(fields: &mut ParsedFields, event_name: &str, name: &str, value: String) {
    match name {
        "Usr" => fields.usr = value,
        "SessionID" => fields.session_id = value,
        "Context" => fields.context = value,
        "CpuTime" => fields.cpu_time = value,
        "Func" => fields.func = value,
        "IName" if event_name == "CALL" => fields.iname = value,
        "MName" if event_name == "CALL" => fields.mname = value,
        "Module" if event_name == "CALL" => fields.module = value,
        "Method" if event_name == "CALL" => fields.method = value,
        "Form" if event_name == "CALL" => fields.form = value,
        "FormItem" if event_name == "CALL" => fields.form_item = value,
        "Sdbl" if event_name == "SDBL" => fields.sdbl = value,
        "tableName" if event_name == "SDBL" => fields.table_name = value,
        _ => {}
    }
}

fn read_field_value(record: &str, start: usize) -> (String, usize) {
    if start >= record.len() {
        return (String::new(), record.len());
    }
    let bytes = record.as_bytes();
    let quote = bytes[start];
    if quote == b'\'' || quote == b'"' {
        let mut out = String::new();
        let mut segment_start = start + 1;
        let mut pos = start + 1;
        while pos < bytes.len() {
            if bytes[pos] == quote {
                if pos + 1 < bytes.len() && bytes[pos + 1] == quote {
                    out.push_str(&record[segment_start..=pos]);
                    pos += 2;
                    segment_start = pos;
                    continue;
                }
                out.push_str(&record[segment_start..pos]);
                return (out, pos + 1);
            }
            pos += 1;
        }
        out.push_str(&record[segment_start..]);
        return (out, record.len());
    }

    let end = find_byte(bytes, start, b',').unwrap_or(record.len());
    (record[start..end].trim().to_string(), end)
}

fn skip_field_value(record: &str, start: usize) -> usize {
    if start >= record.len() {
        return record.len();
    }
    let bytes = record.as_bytes();
    let quote = bytes[start];
    if quote == b'\'' || quote == b'"' {
        let mut pos = start + 1;
        while pos < bytes.len() {
            if bytes[pos] == quote {
                if pos + 1 < bytes.len() && bytes[pos + 1] == quote {
                    pos += 2;
                    continue;
                }
                return pos + 1;
            }
            pos += 1;
        }
        return record.len();
    }
    find_byte(bytes, start, b',').unwrap_or(record.len())
}

fn context_bounds(context: &str) -> (String, String) {
    let normalized = context.replace('\r', "\n");
    let mut first = String::new();
    let mut last = String::new();
    for line in normalized.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if first.is_empty() {
            first = trimmed.to_string();
        }
        last = trimmed.to_string();
    }
    (first, last)
}

fn call_place(
    last_context_line: &str,
    module: &str,
    method: &str,
    func: &str,
    form: &str,
    form_item: &str,
    iname: &str,
    mname: &str,
) -> String {
    if !last_context_line.trim().is_empty() {
        return last_context_line.trim().to_string();
    }
    let module = module.trim();
    let method = method.trim();
    if !module.is_empty() {
        if method.is_empty() {
            return module.to_string();
        }
        return format!("{module}.{method}");
    }

    let mut tail = Vec::new();
    for part in [form, form_item, iname, mname, method] {
        let part = part.trim();
        if !part.is_empty() {
            tail.push(part);
        }
    }
    let tail = tail.join(" ");
    let func = func.trim();
    match (func.is_empty(), tail.is_empty()) {
        (false, false) => format!("{func}: {tail}"),
        (false, true) => func.to_string(),
        (true, false) => tail,
        (true, true) => String::new(),
    }
}

fn sdbl_place(last_context_line: &str, func: &str, table_name: &str) -> String {
    if !last_context_line.trim().is_empty() {
        return last_context_line.trim().to_string();
    }
    [func.trim(), table_name.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_number_u64(value: &str) -> u64 {
    let normalized = value.trim().replace(',', ".");
    if normalized.is_empty() {
        return 0;
    }
    normalized
        .parse::<u64>()
        .or_else(|_| normalized.parse::<f64>().map(|v| v.max(0.0) as u64))
        .unwrap_or(0)
}

fn file_date(path: &Path) -> Option<FileDate> {
    let stem = path.file_stem()?.to_string_lossy();
    let digits = stem.get(0..8)?;
    if !digits.as_bytes().iter().all(u8::is_ascii_digit) {
        return None;
    }
    let year = 2000 + digits[0..2].parse::<i32>().ok()?;
    let month = digits[2..4].parse::<u32>().ok()?;
    let day = digits[4..6].parse::<u32>().ok()?;
    let hour = digits[6..8].parse::<u32>().ok()?;
    Some(FileDate {
        year,
        month,
        day,
        hour,
        text_prefix: format!("{day:02}.{month:02}.{year:04} {hour:02}:"),
    })
}

fn event_datetime(date: Option<&FileDate>, time: &str) -> String {
    let Some(date) = date else {
        return "1970-01-01 00:00:00.000000".to_string();
    };
    let minute = time
        .get(0..2)
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let second = time
        .get(3..5)
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let micro = time
        .find('.')
        .map(|dot| {
            let mut frac = time[dot + 1..].chars().take(6).collect::<String>();
            while frac.len() < 6 {
                frac.push('0');
            }
            frac.parse::<u32>().unwrap_or(0)
        })
        .unwrap_or(0);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06}",
        date.year, date.month, date.day, date.hour, minute, second, micro
    )
}

fn collect_log_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if root.is_file() {
        if is_log_file(root) {
            files.push(root.to_path_buf());
        }
        return Ok(files);
    }
    collect_log_files_recursive(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_log_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_log_files_recursive(&path, files)?;
        } else if is_log_file(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_log_file(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.to_string_lossy().eq_ignore_ascii_case("log"))
        .unwrap_or(false)
}

fn count_lines(path: &Path) -> Result<u64, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut buf = Vec::with_capacity(8192);
    let mut lines = 0u64;
    loop {
        buf.clear();
        let read = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        lines += 1;
    }
    Ok(lines)
}

fn strip_utf8_bom(value: &str) -> &str {
    value.strip_prefix('\u{feff}').unwrap_or(value)
}

fn trim_newline_bytes(buf: &mut Vec<u8>) {
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
}

struct InsertBatch {
    rows: usize,
    body: String,
}

impl InsertBatch {
    fn new(cfg: &Config) -> Self {
        Self {
            rows: 0,
            body: String::with_capacity(cfg.batch_bytes),
        }
    }

    fn push(
        &mut self,
        event: &ParsedEvent,
        cfg: &Config,
        stats: &mut ImportStats,
    ) -> Result<(), String> {
        match cfg.insert_format {
            InsertFormat::Tsv => event.write_tsv(&mut self.body),
            InsertFormat::Json => event.write_json(&mut self.body),
        }
        self.body.push('\n');
        self.rows += 1;
        if self.rows >= cfg.batch_rows || self.body.len() >= cfg.batch_bytes {
            self.flush(cfg, stats)?;
        }
        Ok(())
    }

    fn flush(&mut self, cfg: &Config, stats: &mut ImportStats) -> Result<(), String> {
        if self.rows == 0 {
            return Ok(());
        }
        let format = match cfg.insert_format {
            InsertFormat::Tsv => "TabSeparated",
            InsertFormat::Json => "JSONEachRow",
        };
        let sql = format!(
            "INSERT INTO {}.events ({}) FORMAT {}",
            ident(&cfg.database),
            INSERT_COLUMNS.join(", "),
            format
        );
        clickhouse_post(cfg, &sql, self.body.as_bytes())?;
        stats.inserted += self.rows as u64;
        self.rows = 0;
        self.body.clear();
        Ok(())
    }
}

const INSERT_COLUMNS: [&str; 22] = [
    "event_key",
    "line_no",
    "usr",
    "session_id",
    "event_name",
    "event_time_text",
    "event_dt",
    "duration_us",
    "cpu_time_us",
    "func",
    "form",
    "form_item",
    "iname",
    "mname",
    "method",
    "module",
    "place",
    "first_context_line",
    "query_text",
    "stack_text",
    "file_path",
    "raw_record",
];

impl ParsedEvent {
    fn write_tsv(&self, out: &mut String) {
        write_tsv_str(out, &self.event_key, false);
        write_tsv_u64(out, self.line_no, true);
        write_tsv_str(out, &self.usr, true);
        write_tsv_str(out, &self.session_id, true);
        write_tsv_str(out, &self.event_name, true);
        write_tsv_str(out, &self.event_time_text, true);
        write_tsv_str(out, &self.event_dt, true);
        write_tsv_u64(out, self.duration_us, true);
        write_tsv_u64(out, self.cpu_time_us, true);
        write_tsv_str(out, &self.func, true);
        write_tsv_str(out, &self.form, true);
        write_tsv_str(out, &self.form_item, true);
        write_tsv_str(out, &self.iname, true);
        write_tsv_str(out, &self.mname, true);
        write_tsv_str(out, &self.method, true);
        write_tsv_str(out, &self.module, true);
        write_tsv_str(out, &self.place, true);
        write_tsv_str(out, &self.first_context_line, true);
        write_tsv_str(out, &self.query_text, true);
        write_tsv_str(out, &self.stack_text, true);
        write_tsv_str(out, &self.file_path, true);
        write_tsv_str(out, &self.raw_record, true);
    }

    fn write_json(&self, out: &mut String) {
        out.push('{');
        write_json_str(out, "event_key", &self.event_key, false);
        write_json_u64(out, "line_no", self.line_no, true);
        write_json_str(out, "usr", &self.usr, true);
        write_json_str(out, "session_id", &self.session_id, true);
        write_json_str(out, "event_name", &self.event_name, true);
        write_json_str(out, "event_time_text", &self.event_time_text, true);
        write_json_str(out, "event_dt", &self.event_dt, true);
        write_json_u64(out, "duration_us", self.duration_us, true);
        write_json_u64(out, "cpu_time_us", self.cpu_time_us, true);
        write_json_str(out, "func", &self.func, true);
        write_json_str(out, "form", &self.form, true);
        write_json_str(out, "form_item", &self.form_item, true);
        write_json_str(out, "iname", &self.iname, true);
        write_json_str(out, "mname", &self.mname, true);
        write_json_str(out, "method", &self.method, true);
        write_json_str(out, "module", &self.module, true);
        write_json_str(out, "place", &self.place, true);
        write_json_str(out, "first_context_line", &self.first_context_line, true);
        write_json_str(out, "query_text", &self.query_text, true);
        write_json_str(out, "stack_text", &self.stack_text, true);
        write_json_str(out, "file_path", &self.file_path, true);
        write_json_str(out, "raw_record", &self.raw_record, true);
        out.push('}');
    }
}

fn write_tsv_str(out: &mut String, value: &str, tab: bool) {
    if tab {
        out.push('\t');
    }
    escape_tsv_string(value, out);
}

fn write_tsv_u64(out: &mut String, value: u64, tab: bool) {
    if tab {
        out.push('\t');
    }
    let _ = write!(out, "{value}");
}

fn escape_tsv_string(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
}

fn write_json_str(out: &mut String, key: &str, value: &str, comma: bool) {
    if comma {
        out.push(',');
    }
    let _ = write!(out, "\"{key}\":\"");
    escape_json_string(value, out);
    out.push('"');
}

fn write_json_u64(out: &mut String, key: &str, value: u64, comma: bool) {
    if comma {
        out.push(',');
    }
    let _ = write!(out, "\"{key}\":{value}");
}

fn escape_json_string(value: &str, out: &mut String) {
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c <= '\u{1f}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(str::trim)
        .filter(|stmt| !stmt.is_empty() && !stmt.starts_with("--"))
        .map(str::to_string)
        .collect()
}

fn clickhouse_query(cfg: &Config, sql: &str) -> Result<(), String> {
    clickhouse_post(cfg, sql, &[])
}

fn clickhouse_post(cfg: &Config, sql: &str, body: &[u8]) -> Result<(), String> {
    let mut stream = TcpStream::connect((cfg.host.as_str(), cfg.port))
        .map_err(|e| format!("connect {}:{} failed: {e}", cfg.host, cfg.port))?;
    let path = format!("/?query={}", url_encode(sql));
    let auth = base64_encode(format!("{}:{}", cfg.user, cfg.password).as_bytes());
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Basic {auth}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        cfg.host,
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|e| format!("write request failed: {e}"))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("read response failed: {e}"))?;
    let response_text = String::from_utf8_lossy(&response);
    if response_text.starts_with("HTTP/1.1 200") || response_text.starts_with("HTTP/1.0 200") {
        return Ok(());
    }
    Err(response_text.into_owned())
}

fn url_encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = bytes.get(i + 1).copied().unwrap_or(0);
        let b2 = bytes.get(i + 2).copied().unwrap_or(0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < bytes.len() {
            out.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

fn ident(value: &str) -> String {
    assert!(
        value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "unsafe ClickHouse identifier"
    );
    value.to_string()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date() -> FileDate {
        FileDate {
            year: 2026,
            month: 6,
            day: 17,
            hour: 15,
            text_prefix: "17.06.2026 15:".to_string(),
        }
    }

    #[test]
    fn parses_call_with_context_place() {
        let record = "00:00.010001-14922,CALL,1,Usr=u,SessionID=1,Context='A\nB',CpuTime=15625";
        let event = parse_record(record, "a.log", 42, Some(&date()), false).unwrap();
        assert_eq!(event.event_name, "CALL");
        assert_eq!(event.line_no, 42);
        assert_eq!(event.usr, "u");
        assert_eq!(event.session_id, "1");
        assert_eq!(event.place, "B");
        assert_eq!(event.first_context_line, "A");
        assert_eq!(event.duration_us, 14922);
        assert_eq!(event.cpu_time_us, 15625);
        assert_eq!(event.event_dt, "2026-06-17 15:00:00.010001");
    }

    #[test]
    fn call_place_uses_module_method_then_fallback() {
        let by_module = "00:00.010001-1,CALL,1,Module=Doc.Module,Method=Run,Func=Ignored";
        let event = parse_record(by_module, "a.log", 1, Some(&date()), false).unwrap();
        assert_eq!(event.place, "Doc.Module.Run");

        let fallback =
            "00:00.010001-1,CALL,1,Func=Call,Form=F,FormItem=I,IName=IN,MName=MN,Method=M";
        let event = parse_record(fallback, "a.log", 1, Some(&date()), false).unwrap();
        assert_eq!(event.place, "Call: F I IN MN M");
    }

    #[test]
    fn parses_multiple_typed_fields() {
        let record = "00:00.010001-1,CALL,1,Usr=u,SessionID=1,Func=Call,Form=F,FormItem=I,IName=IN,MName=MN,Method=M,CpuTime=12";
        let event = parse_record(record, "a.log", 5, Some(&date()), false).unwrap();
        assert_eq!(event.usr, "u");
        assert_eq!(event.session_id, "1");
        assert_eq!(event.func, "Call");
        assert_eq!(event.form, "F");
        assert_eq!(event.form_item, "I");
        assert_eq!(event.iname, "IN");
        assert_eq!(event.mname, "MN");
        assert_eq!(event.method, "M");
        assert_eq!(event.cpu_time_us, 12);
        assert_eq!(event.place, "Call: F I IN MN M");
    }

    #[test]
    fn sdbl_never_uses_query_as_place() {
        let record = "00:00.010022-8,SDBL,2,Func=Select,tableName=Catalog,Sdbl='SELECT * FROM T'";
        let event = parse_record(record, "a.log", 1, Some(&date()), false).unwrap();
        assert_eq!(event.place, "Select Catalog");
        assert_eq!(event.query_text, "SELECT * FROM T");
    }

    #[test]
    fn parses_short_sdbl_without_fields() {
        let record = "00:01.588049-9,SDBL,2";
        let event = parse_record(record, "a.log", 1, Some(&date()), false).unwrap();
        assert_eq!(event.event_name, "SDBL");
        assert_eq!(event.duration_us, 9);
        assert_eq!(event.cpu_time_us, 0);
        assert_eq!(event.event_time_text, "17.06.2026 15:00:01.588049");
        assert_eq!(event.event_dt, "2026-06-17 15:00:01.588049");
        assert_eq!(event.place, "");
        assert_eq!(event.query_text, "");
        assert_eq!(event.stack_text, "");
    }

    #[test]
    fn reads_doubled_quotes() {
        let record = "00:00.010022-8,SDBL,2,Context=\"A \"\"quoted\"\" line\",Sdbl='select'";
        let event = parse_record(record, "a.log", 1, Some(&date()), false).unwrap();
        assert_eq!(event.place, "A \"quoted\" line");
    }

    #[test]
    fn tsv_escapes_row_breaking_chars() {
        let record = "00:00.010022-8,SDBL,2,Usr=u,SessionID=1,Context='A\nB\tC',Sdbl='select'";
        let event = parse_record(record, "a.log", 1, Some(&date()), true).unwrap();
        let mut row = String::new();
        event.write_tsv(&mut row);
        assert!(!row.contains('\n'));
        assert!(row.starts_with("a.log|1\t1\tu\t1\tSDBL\t"));
        assert!(row.contains("A\\nB\\tC"));
    }

    #[test]
    fn json_includes_line_no_usr_and_session_id() {
        let record = "00:00.010022-8,SDBL,2,Usr=u,SessionID=1,Sdbl='select'";
        let event = parse_record(record, "a.log", 77, Some(&date()), false).unwrap();
        let mut row = String::new();
        event.write_json(&mut row);
        assert!(row.contains("\"event_key\":\"a.log|77\""));
        assert!(row.contains("\"line_no\":77"));
        assert!(row.contains("\"usr\":\"u\""));
        assert!(row.contains("\"session_id\":\"1\""));
    }

    #[test]
    fn detects_only_record_headers_at_line_start() {
        assert_eq!(
            event_name_at_record_start("00:00.010001-14922,CALL,1,Usr=u"),
            Some("CALL".to_string())
        );
        assert_eq!(
            event_name_at_record_start("\u{feff}00:00.010001-14922,CALL,1,Usr=u"),
            Some("CALL".to_string())
        );
        assert_eq!(event_name_at_record_start("not a header"), None);
    }
}
