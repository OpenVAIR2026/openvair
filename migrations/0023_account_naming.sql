ALTER TABLE organizations RENAME TO accounts;

ALTER TABLE memberships RENAME COLUMN organization_id TO account_id;
ALTER TABLE channels RENAME COLUMN organization_id TO account_id;
ALTER TABLE model_routes RENAME COLUMN organization_id TO account_id;
ALTER TABLE virtual_api_keys RENAME COLUMN organization_id TO account_id;
ALTER TABLE usage_events RENAME COLUMN organization_id TO account_id;
ALTER TABLE request_logs RENAME COLUMN organization_id TO account_id;
ALTER TABLE audit_logs RENAME COLUMN organization_id TO account_id;
ALTER TABLE invitations RENAME COLUMN organization_id TO account_id;

ALTER TABLE accounts RENAME CONSTRAINT organizations_pkey TO accounts_pkey;
ALTER TABLE accounts RENAME CONSTRAINT organizations_api_key_prefix_format TO accounts_api_key_prefix_format;
ALTER TABLE memberships RENAME CONSTRAINT memberships_organization_id_fkey TO memberships_account_id_fkey;
ALTER TABLE channels RENAME CONSTRAINT channels_organization_id_fkey TO channels_account_id_fkey;
ALTER TABLE model_routes RENAME CONSTRAINT model_routes_organization_id_fkey TO model_routes_account_id_fkey;
ALTER TABLE virtual_api_keys RENAME CONSTRAINT virtual_api_keys_organization_id_fkey TO virtual_api_keys_account_id_fkey;
ALTER TABLE usage_events RENAME CONSTRAINT usage_events_organization_id_fkey TO usage_events_account_id_fkey;
ALTER TABLE request_logs RENAME CONSTRAINT request_logs_organization_id_fkey TO request_logs_account_id_fkey;
ALTER TABLE audit_logs RENAME CONSTRAINT audit_logs_organization_id_fkey TO audit_logs_account_id_fkey;
ALTER TABLE invitations RENAME CONSTRAINT invitations_organization_id_fkey TO invitations_account_id_fkey;

ALTER INDEX channels_org_idx RENAME TO channels_account_idx;
ALTER INDEX routes_org_model_idx RENAME TO routes_account_model_idx;
ALTER INDEX usage_org_created_idx RENAME TO usage_account_created_idx;
ALTER INDEX audit_org_created_idx RENAME TO audit_account_created_idx;
