-- A message's text part is now labelled format=flowed; delsp=no, and every draft's text is
-- written as flowed text. A plain draft stored before that (no HTML: made by `mailo compose`, a
-- reply or a forward not yet opened in the composer) holds its text as it was written, so sent
-- as it is, a line ending in a space would join the next and a leading space would be lost. A
-- plain template kept from such a draft passes its text to every draft started from it.
--
-- So every such draft and template is queued here, and the next `mailo` run rewrites its text
-- as flowed text with every line hard. Tables rather than SQL, for the reason 0007 gives: the
-- rewrite is Rust this crate cannot see. One with HTML was written by the composer, whose text
-- was flowed already.

CREATE TABLE drafts_to_reflow (
    draft TEXT PRIMARY KEY REFERENCES drafts(id) ON DELETE CASCADE
);
INSERT INTO drafts_to_reflow (draft)
    SELECT id FROM drafts WHERE body_html IS NULL;

CREATE TABLE templates_to_reflow (
    template TEXT PRIMARY KEY REFERENCES templates(id) ON DELETE CASCADE
);
INSERT INTO templates_to_reflow (template)
    SELECT id FROM templates WHERE body_html IS NULL;
