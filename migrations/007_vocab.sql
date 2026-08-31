CREATE TABLE vocab_cards (
    id INTEGER PRIMARY KEY,
    user_phone TEXT NOT NULL,
    language TEXT NOT NULL,
    front TEXT NOT NULL,
    back TEXT NOT NULL,
    partner_phone TEXT,
    conversation_id INTEGER REFERENCES conversations(id),
    interval_days REAL NOT NULL DEFAULT 0,
    ease_factor REAL NOT NULL DEFAULT 2.5,
    repetitions INTEGER NOT NULL DEFAULT 0,
    due_date TEXT NOT NULL DEFAULT (date('now')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (user_phone, language, front, back)
);

CREATE INDEX vocab_cards_user_language_due_idx
    ON vocab_cards (user_phone, language, due_date);
