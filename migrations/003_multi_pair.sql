-- Per-user settings (active conversation for future multi-convo support).
CREATE TABLE user_settings (
    phone TEXT PRIMARY KEY,
    active_conversation_id INTEGER REFERENCES conversations(id),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Multi-step WhatsApp onboarding before a user is registered.
CREATE TABLE onboarding_sessions (
    phone TEXT PRIMARY KEY,
    step TEXT NOT NULL,
    data_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Pairing invites between two phone numbers.
CREATE TABLE conversation_invites (
    id INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    inviter_phone TEXT NOT NULL,
    invitee_phone TEXT NOT NULL,
    inviter_role TEXT NOT NULL CHECK (inviter_role IN ('teacher', 'learner')),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'accepted', 'declined')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX conversation_invites_invitee_status_idx
    ON conversation_invites (invitee_phone, status);

CREATE INDEX conversation_invites_conversation_idx
    ON conversation_invites (conversation_id);

-- Allow the same phone in multiple conversations (future), but only once per conversation.
CREATE TABLE participants_new (
    id INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    phone TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('teacher', 'learner')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (conversation_id, phone),
    UNIQUE (conversation_id, role)
);

INSERT INTO participants_new (id, conversation_id, phone, role, created_at)
SELECT id, conversation_id, phone, role, created_at FROM participants;

DROP TABLE participants;

ALTER TABLE participants_new RENAME TO participants;

CREATE INDEX participants_phone_idx ON participants (phone);
CREATE INDEX participants_conversation_id_idx ON participants (conversation_id);
