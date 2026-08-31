PRAGMA foreign_keys=OFF;

CREATE TABLE conversations_new (
    id INTEGER PRIMARY KEY,
    mode TEXT NOT NULL DEFAULT 'tutor'
        CHECK (mode IN ('tutor', 'exchange', 'exchange_turns')),
    target_language TEXT NOT NULL DEFAULT 'Norwegian',
    source_language TEXT NOT NULL DEFAULT 'English',
    exchange_turn_phone TEXT,
    exchange_awaiting_reply INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO conversations_new (id, mode, target_language, source_language, created_at)
SELECT id, mode, target_language, source_language, created_at
FROM conversations;

DROP TABLE conversations;

ALTER TABLE conversations_new RENAME TO conversations;

PRAGMA foreign_keys=ON;
