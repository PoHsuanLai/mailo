-- The indexed text is now `mail_domain::filter::search_tokens`, already folded, rather than the
-- store's own copy of that tokenizer with folding left to FTS5 (phase 9.3). Rows indexed the old
-- way still match most queries, but not all — a Hebrew vowel point used to join a word and now
-- ends one — so every row is reindexed. Same two statements as 0005, for the same reasons.

UPDATE messages SET fts_text = NULL;
INSERT INTO messages_fts(messages_fts) VALUES ('delete-all');
