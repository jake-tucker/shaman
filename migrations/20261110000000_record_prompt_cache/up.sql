-- When each Session's Prompt Cache lapses, and the Lapse Penalty forecast for
-- its next Prompt once it has, as its latest Turn left them, so a restarted
-- Server still knows which Sessions are cold. Nullable: a Session whose
-- Provider has no Cache Lifetime, or whose cache no Turn has written, carries
-- none, which is every Session that predates this column.
ALTER TABLE sessions ADD COLUMN prompt_cache TEXT;
