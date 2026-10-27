-- What the server was last observed to support, kept apart from `caps`, which has the folders
-- the person picked for roles (`account_prefs.folder_roles`) laid over it. A role given back to
-- Automatic is read from here, so it plays the folder the server named at once rather than none
-- until the next refresh. A row written before this has only `caps`, which is the best there is.
ALTER TABLE account_caps ADD COLUMN observed TEXT;
UPDATE account_caps SET observed = caps;
