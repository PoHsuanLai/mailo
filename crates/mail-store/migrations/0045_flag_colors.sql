-- Flag colours: Apple Mail's seven, kept with `\Flagged` as the keywords `$MailFlagBit0` to
-- `$MailFlagBit2`. serde(Hue): '"red"' | '"orange"' | … | '"gray"'.
--
-- On `messages`, beside `star`, and in the `thread_summary` cache as the newest starred message's
-- colour. A colour means something only while the message is starred. Every flag that exists
-- before this migration was a plain `\Flagged`, which is a red flag, so the default is the true
-- value for every existing row and not a guess.
ALTER TABLE messages ADD COLUMN flag TEXT NOT NULL DEFAULT '"red"';
ALTER TABLE thread_summary ADD COLUMN flag TEXT NOT NULL DEFAULT '"red"';
