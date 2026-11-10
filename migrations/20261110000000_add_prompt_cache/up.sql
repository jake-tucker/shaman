-- When each Session's Prompt Cache lapses, and the Model whose call wrote it,
-- as its latest Turn left them, so a restarted Server still knows which
-- Sessions are cold. The Lapse Penalty is a forecast and is never stored.
-- Nullable: a Session whose Provider has no Cache Lifetime, or whose cache no
-- Turn has written, carries none, which is every Session that predates this
-- column.
ALTER TABLE sessions ADD COLUMN prompt_cache TEXT;
