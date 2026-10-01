CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS control_snapshot (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    payload TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS state_transitions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    previous TEXT NOT NULL,
    next TEXT NOT NULL,
    reason TEXT NOT NULL,
    actor TEXT NOT NULL,
    at TEXT NOT NULL,
    detail TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS alerts (
    id TEXT PRIMARY KEY,
    dedup_key TEXT NOT NULL,
    resolved INTEGER NOT NULL,
    payload TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
