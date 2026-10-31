-- 0041 shipped the feature as "sharing"; the UI and the domain model both call it
-- a grant now (a key owner grants another account the right to use one key as a
-- channel, and can cancel it). Rename the storage so the code stops translating.
ALTER TABLE key_shares RENAME TO key_grants;
ALTER TABLE channels RENAME COLUMN key_share_id TO key_grant_id;

-- Postgres keeps index names through a table rename; align them with the table.
ALTER INDEX key_shares_active_key_idx RENAME TO key_grants_active_key_idx;
ALTER INDEX key_shares_pending_email_idx RENAME TO key_grants_pending_email_idx;
ALTER INDEX channels_source_key_idx RENAME TO channels_key_grant_source_idx;
