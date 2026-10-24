-- The largest message the account's sending server said it takes, in octets as they go on the
-- wire: SMTP's `SIZE` (RFC 1870), heard in the `EHLO` of a submission, or the JMAP session's
-- upload limits. NULL when the server stated none, or has not been asked yet.
--
-- Beside the capabilities, and not in their JSON, because the two are learned from different
-- servers at different times: a sync writes the whole capabilities row from the incoming server,
-- and would otherwise erase what the last submission heard.
ALTER TABLE account_caps ADD COLUMN send_limit INTEGER;
