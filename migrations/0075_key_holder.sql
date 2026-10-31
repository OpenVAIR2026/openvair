-- Who a key was issued to, for the admin's own records.
ALTER TABLE virtual_api_keys ADD COLUMN holder text;
