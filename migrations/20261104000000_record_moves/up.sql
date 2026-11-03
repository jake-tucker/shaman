-- An Activity's Turn reference becomes optional: a Move happens between
-- Turns, so it belongs to none (ADR 0059). SQLite cannot drop NOT NULL in
-- place, so the table is rebuilt around the loosened column with every row
-- carried over.
CREATE TABLE activities_with_optional_turn (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    turn_id TEXT,
    row_order BIGINT NOT NULL,
    transcript_order BIGINT NOT NULL,
    payload TEXT NOT NULL
);

INSERT INTO activities_with_optional_turn SELECT * FROM activities;
DROP TABLE activities;
ALTER TABLE activities_with_optional_turn RENAME TO activities;

CREATE INDEX activities_session_order_idx ON activities(session_id, row_order);
CREATE UNIQUE INDEX activities_transcript_order_idx
    ON activities(session_id, transcript_order);
