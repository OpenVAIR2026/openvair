-- 0043 dropped key_grants and channels.key_grant_id but kept the channels that
-- in-app grants had created. The gateway used to skip them once the owner
-- cancelled the grant; without the column they routed again, and recipients
-- who never held the key kept access the owner can no longer revoke per
-- account. A VAIR key channel must come from pasting the key, so remove every
-- channel a grant created.
--
-- The grant link is gone, so the audit log identifies these channels: they were
-- created with via='share' (0041) or via='grant' (0042). A channel whose key
-- was later replaced by pasting another key is legitimate and stays.
WITH removed AS (
    DELETE FROM channels c
    WHERE c.kind='vair_key'
      AND EXISTS (
          SELECT 1 FROM audit_logs a
          WHERE a.action='channel.created' AND a.resource_type='channel' AND a.resource_id=c.id
            AND a.metadata->>'via' IN ('share','grant'))
      AND NOT EXISTS (
          SELECT 1 FROM audit_logs a
          WHERE a.action='channel.updated' AND a.resource_type='channel' AND a.resource_id=c.id
            AND a.metadata->>'source_key_replaced'='true')
    RETURNING c.id, c.account_id, c.source_key_id
)
INSERT INTO audit_logs(account_id,user_id,action,resource_type,resource_id,metadata)
SELECT account_id, NULL, 'channel.deleted', 'channel', id,
       jsonb_build_object('reason','key_grant_removed','source_key_id',source_key_id)
FROM removed;
