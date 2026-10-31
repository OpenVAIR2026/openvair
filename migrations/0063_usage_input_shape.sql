-- Local token counts of request text by part (prose, tool traffic, thinking,
-- tool definitions) and the request's structure, under the tokenizer of the
-- upstream model. Passive metering adds the message frame and compares the
-- result with the reported input tokens. NULL when the tokenizer is unknown or
-- the request carries content text cannot predict.
ALTER TABLE usage_events ADD COLUMN input_shape JSONB;

-- Reported input tokens against the local counts of the same 24 hours:
-- median ratio and excess, and how the excess grows with request size.
-- Recorded only until checked on live traffic.
ALTER TABLE channel_passive_stats ADD COLUMN exact JSONB;
