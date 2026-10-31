-- Existing hashes cannot be reversed; only newly created keys are recoverable.
ALTER TABLE virtual_api_keys ADD COLUMN secret_ciphertext TEXT;
