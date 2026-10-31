-- Each user owns exactly one account; member invitations and roles are retired.
-- Stop rather than silently granting full access to a non-owner membership.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM memberships WHERE role <> 'owner') THEN
        RAISE EXCEPTION 'non-owner memberships exist; resolve them before this migration';
    END IF;
END $$;

ALTER TABLE memberships
    ADD CONSTRAINT memberships_one_account_per_user UNIQUE (user_id),
    ADD CONSTRAINT memberships_one_user_per_account UNIQUE (account_id);

-- The role column stays, fixed to owner, so earlier releases keep working after
-- a rollback. A later migration can drop it once rollback is no longer needed.
ALTER TABLE memberships ALTER COLUMN role SET DEFAULT 'owner';
ALTER TABLE memberships ADD CONSTRAINT memberships_owner_only CHECK (role = 'owner');

DROP TABLE invitations;
