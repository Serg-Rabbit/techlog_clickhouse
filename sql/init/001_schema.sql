CREATE DATABASE IF NOT EXISTS techlog;

CREATE TABLE IF NOT EXISTS techlog.events
(
    event_key String,
    file_path String,
    line_no UInt64,
    event_dt DateTime64(6, 'Asia/Krasnoyarsk'),
    place String,
    event_name LowCardinality(String),
    event_time_text String,
    duration_us UInt64,
    duration_sec Float64 MATERIALIZED duration_us / 1000000,
    cpu_time_us UInt64,
    cpu_time_sec Float64 MATERIALIZED cpu_time_us / 1000000,
    func String,
    form String,
    form_item String,
    iname String,
    mname String,
    method String,
    module String,
    first_context_line String,
    query_text String,
    stack_text String,
    usr String,
    session_id String,
    raw_record String
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(event_dt)
ORDER BY (event_dt, event_name, place)
TTL event_dt + INTERVAL 1 MONTH;

ALTER TABLE techlog.events
    ADD COLUMN IF NOT EXISTS usr String AFTER stack_text;

ALTER TABLE techlog.events
    ADD COLUMN IF NOT EXISTS session_id String AFTER usr;

ALTER TABLE techlog.events
    MODIFY TTL event_dt + INTERVAL 1 MONTH;
