-- Contact groups (`crate::contact::group`): a vCard `KIND:group` and its `MEMBER` URIs, made here
-- or synced from a CardDAV address book.
--
-- id:      `local:<uid>` for a group made here, the card's URL for a synced one.
-- uid:     the card's `UID`, which another group's `MEMBER` may name it by.
-- name:    what typing in To finds it by.
-- members: serde(Vec<String>): the `MEMBER` URIs in order, as written, resolved or not.
-- home:    serde(GroupHome): local, or the book and card it syncs with and whether it holds an
--          edit the server has not been sent.
CREATE TABLE contact_groups (
    id      TEXT PRIMARY KEY,
    uid     TEXT,
    name    TEXT NOT NULL,
    members TEXT NOT NULL,
    home    TEXT NOT NULL
) WITHOUT ROWID;
