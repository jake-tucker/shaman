-- The Session emoji feature is retired in favor of a Shaman-owned Icon
-- Catalog (see ADR 0028); no Session carries an emoji any longer.
ALTER TABLE sessions DROP COLUMN emoji;
