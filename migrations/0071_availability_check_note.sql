-- What the last scheduled or manual check had to work around, such as a test
-- model the upstream no longer serves. Only checks change it, not traffic.
ALTER TABLE channel_health ADD COLUMN check_note TEXT;
