ALTER TABLE accounts ALTER COLUMN channel_limit SET DEFAULT 20;
UPDATE accounts SET channel_limit = 20 WHERE channel_limit < 20;
