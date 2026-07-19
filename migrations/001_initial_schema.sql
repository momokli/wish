CREATE TABLE IF NOT EXISTS submissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    spotify_url TEXT NOT NULL,
    track_title TEXT,
    track_artist TEXT,
    cover_url TEXT,
    source TEXT NOT NULL DEFAULT 'spotify',
    status TEXT NOT NULL DEFAULT 'pending',
    filename TEXT,
    file_size INTEGER,
    error_message TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    updated_at INTEGER DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS idx_submissions_status ON submissions(status);
CREATE INDEX IF NOT EXISTS idx_submissions_created ON submissions(created_at);
