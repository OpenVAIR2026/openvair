-- The in-app grant path is gone: a VAIR key channel is now created only by
-- pasting the source key, so there is no invite row and no per-channel link to
-- one. Existing channels survive on `source_key_id` alone and keep routing.

-- Drop the column first: it carries the foreign key into key_grants, and 0042
-- renamed the column without renaming that constraint, so the table cannot go
-- while it is still referenced.
ALTER TABLE channels DROP COLUMN key_grant_id;

DROP TABLE key_grants;

-- 0042 renamed this index along with the table it happened to sit next to; the
-- column it actually covers never had anything to do with grants.
ALTER INDEX channels_key_grant_source_idx RENAME TO channels_source_key_idx;
