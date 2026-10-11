-- Accounts of the desktop's accountd go: mailo keeps its own accounts and credentials.
--
-- Earlier builds could link to accountd and keep its accounts here with a plan whose auth was
-- `granted` (a grant on an account whose secrets accountd held, never mailo). That mode is gone,
-- and so is the variant: such a plan no longer reads as an `AccountPlan`, and a row of it would
-- be an account nothing can sign in to. So it is removed the way removing an account removes
-- one: its row goes, and every table that names it follows by `ON DELETE CASCADE` (contacts by
-- `SET NULL`), which `migrate` turns on before any migration runs. Blobs only it used are left on
-- disk: they cost space, not mail, and no row names them. A plan that is not JSON is not touched.
DELETE FROM accounts
WHERE json_valid(plan) AND json_extract(plan, '$.auth.kind') = 'granted';
