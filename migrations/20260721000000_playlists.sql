CREATE TABLE IF NOT EXISTS playlists (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL UNIQUE,
    source TEXT NOT NULL,
    title TEXT,
    track_count INTEGER DEFAULT 0,
    new_since_sync INTEGER DEFAULT 0,
    last_synced INTEGER,
    last_error TEXT,
    created_at INTEGER DEFAULT (unixepoch())
);
