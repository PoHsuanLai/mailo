-- A draft's priority and its request for delivery reports.
--
-- `priority` holds serde(Priority) and `delivery` serde(DeliveryReport). Every draft written
-- before they existed was Normal and asked for no report, which is what the defaults say. The
-- outbox needs no column: a submission's request is frozen into its `op` with the rest of the
-- envelope, and a row written before carries none, which reads as Unrequested.

ALTER TABLE drafts ADD COLUMN priority TEXT NOT NULL DEFAULT '"normal"';
ALTER TABLE drafts ADD COLUMN delivery TEXT NOT NULL DEFAULT '"unrequested"';
