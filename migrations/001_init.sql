CREATE TABLE conversations (
    id INTEGER PRIMARY KEY,
    target_language TEXT NOT NULL DEFAULT 'Norwegian',
    source_language TEXT NOT NULL DEFAULT 'English',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    role TEXT NOT NULL CHECK (role IN ('teacher', 'learner')),
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX messages_conversation_id_idx ON messages (conversation_id);
