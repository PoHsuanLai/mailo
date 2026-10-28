-- Whether an account's labels are written to its server as keywords (`ServerLabels::Keywords`).
-- Off until the person says so, because a label's name is then shared with every other mail
-- program on the account. An account with no row keeps its labels here.
ALTER TABLE account_prefs ADD COLUMN keep_labels INTEGER NOT NULL DEFAULT 0
    CHECK (keep_labels IN (0, 1));
