-- VIPs (`crate::contact::vip`): the people whose mail matters most, marked by hand.
--
-- A mark on a person, keyed as the address book is (`contacts.address`, lower-cased and trimmed),
-- and one list across accounts and Spaces. A table of its own rather than a column of `contacts`:
-- a VIP may be marked before any mail has taught the book their address, and stays marked when
-- the book forgets them and learns them again.
--
-- name: the name they were marked with, for the list when the book has none. NULL for none.
CREATE TABLE vips (
    address TEXT PRIMARY KEY,
    name    TEXT
) WITHOUT ROWID;
