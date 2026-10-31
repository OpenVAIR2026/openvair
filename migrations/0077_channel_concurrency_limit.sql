-- Optional cap on requests in flight through one upstream channel; NULL means no cap.
ALTER TABLE channels ADD COLUMN concurrency_limit integer
  CHECK (concurrency_limit IS NULL OR concurrency_limit BETWEEN 1 AND 10000);
