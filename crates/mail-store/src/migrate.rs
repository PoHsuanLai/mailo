//! Forward-only schema migrations.
//!
//! `accounts` holds a serialized `AccountPlan`, so a field added to that type breaks startup
//! for every existing row unless it carries `#[serde(default)]` and a migration exists. Both
//! are required by `CONVENTIONS.md` section 3.

// wave 2
