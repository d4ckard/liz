CREATE TABLE sessions (
    thread_id TEXT PRIMARY KEY UNIQUE NOT NULL,
    user_id TEXT NOT NULL,
    source_code TEXT NOT NULL
);
