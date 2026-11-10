-- A Turn begun for a Compaction now names who asked for it, the user or Suru
-- ahead of a Prompt Cache lapse, where it said only that one was asked for.
-- Every such Turn stored before was the user's.
UPDATE turns
SET payload = json_set(payload, '$.compaction_requested', 'manual')
WHERE json_type(payload, '$.compaction_requested') = 'true';
UPDATE turns
SET payload = json_remove(payload, '$.compaction_requested')
WHERE json_type(payload, '$.compaction_requested') = 'false';
