-- Saved views: a filter and how to show what it finds (`plan.md` item 19). Local only.
--
-- 0001 made a `views` table with one column per field of `View`, and nothing ever wrote a row
-- to it: no `Store` method read or kept a view before this migration. So it is dropped rather
-- than converted, and no user's view is lost.
--
-- One column holding the whole `View` as its serde form, not a column per field. The serde
-- form is the persisted schema (`CONVENTIONS.md` §3), so a field added to `View` with
-- `#[serde(default)]` needs no migration here, where a column per field needed one each; and
-- the 0001 table had already drifted from its type (`group_by` was commented
-- `Option<Property>` after F35 made it `Option<GroupKey>`).
--
-- `position` is the sidebar's order: a new view goes last, and keeping an existing one again
-- keeps its place.
DROP INDEX views_position;
DROP TABLE views;
CREATE TABLE views (
    id          TEXT PRIMARY KEY,
    view        TEXT NOT NULL,       -- serde(View)
    position    INTEGER NOT NULL
);
CREATE INDEX views_position ON views(position);
