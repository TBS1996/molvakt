CREATE TABLE participants (
    id INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    phone TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL CHECK (role IN ('teacher', 'learner')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE learner_sessions (
    participant_id INTEGER PRIMARY KEY REFERENCES participants(id),
    session_json TEXT NOT NULL DEFAULT '"Idle"',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX participants_conversation_id_idx ON participants (conversation_id);
