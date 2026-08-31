ALTER TABLE conversations ADD COLUMN mode TEXT NOT NULL DEFAULT 'tutor'
    CHECK (mode IN ('tutor', 'exchange'));

ALTER TABLE participants ADD COLUMN learning_language TEXT;

ALTER TABLE messages ADD COLUMN sender_phone TEXT;
