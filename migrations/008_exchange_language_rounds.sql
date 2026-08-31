ALTER TABLE conversations ADD COLUMN exchange_active_language TEXT;
ALTER TABLE conversations ADD COLUMN exchange_round_messages INTEGER NOT NULL DEFAULT 0;
ALTER TABLE conversations ADD COLUMN exchange_round_starter_phone TEXT;
