//! Who an account sends as: its own address under the name the person gives it, and the aliases
//! they add beside it, each with its own name and, if they want one, its own Reply-To.
//!
//! The rows are the `identities` table's, which is what a draft names ([`crate::compose`] reads
//! the same side). The plan keeps a copy (`AccountPlan.identities`), which the vacation reply and
//! the "is this mine" test read, so every change here writes the copy after the table: the two
//! would otherwise disagree from the first alias on.
//!
//! What the person typed comes in as text and is read here, once, so the Settings page and any
//! later caller refuse the same things: an alias needs an address, a Reply-To that is given must
//! be one, and a blank name is no name.

use crate::error::{CoreError, Logged};
use mail_domain::{Address, Identity, IdentityId, IsDefault};
use mail_store::SqliteStore;
use porter_core::AccountId;

/// An identity as the person typed it: a name, an address and a Reply-To, each as written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Typed {
    pub name: String,
    pub email: String,
    pub reply_to: String,
}

impl Typed {
    /// What `identity` says, as its fields would show it.
    pub fn of(identity: &Identity) -> Typed {
        // No name and no Reply-To are empty fields.
        let name = identity.from.name.clone().unwrap_or_default();
        let reply_to = identity.reply_to.as_ref().map(ToString::to_string);
        // As above: none is an empty field.
        let reply_to = reply_to.unwrap_or_default();
        Typed {
            name,
            email: identity.from.email.clone(),
            reply_to,
        }
    }

    /// The `From` this says: the name trimmed, and none when it is blank, which goes out as the
    /// bare address rather than as an empty quoted name.
    fn from(&self) -> Result<Address, CoreError> {
        let email: Address = self.email.trim().parse()?;
        Ok(Address {
            name: name(&self.name),
            email: email.email,
        })
    }

    /// The Reply-To this says: none when the field is blank.
    fn reply_to(&self) -> Result<Option<Address>, CoreError> {
        let text = self.reply_to.trim();
        if text.is_empty() {
            return Ok(None);
        }
        Ok(Some(text.parse()?))
    }
}

/// A typed name as it is kept: trimmed, and none when nothing is left.
fn name(typed: &str) -> Option<String> {
    let trimmed = typed.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The account's identities, its own address first and then its aliases, as the table holds
/// them.
pub fn of_account(store: &SqliteStore, account: AccountId) -> Result<Vec<Identity>, CoreError> {
    Ok(store.identities(account)?)
}

/// Give the account's own address the name its mail goes out under, or take it away with a
/// blank one. The address, Reply-To and signature are left as they are.
pub fn set_name(
    store: &SqliteStore,
    account: AccountId,
    typed: &str,
) -> Result<Identity, CoreError> {
    let own = crate::compose::identity_of(store, account, None)?;
    let from = Address {
        name: name(typed),
        email: own.from.email.clone(),
    };
    store.edit_identity(own.id, &from, own.reply_to.as_ref())?;
    mirrored(store, &own.account);
    Ok(Identity { from, ..own })
}

/// Add an alias to `account`: another address it sends as, with its own name and Reply-To.
///
/// Refused, with nothing written, when the address is not one, when the account already sends
/// as it, and when the account has no identity of its own yet: an alias beside nothing would
/// become the default by being the only row.
pub fn add_alias(
    store: &SqliteStore,
    account: AccountId,
    typed: &Typed,
) -> Result<Identity, CoreError> {
    crate::compose::identity_of(store, account.clone(), None)?;
    let alias = Identity {
        id: IdentityId::generate(),
        account,
        from: typed.from()?,
        reply_to: typed.reply_to()?,
        signature: None,
        default: IsDefault::Alternate,
    };
    store.add_identity(&alias)?;
    mirrored(store, &alias.account);
    Ok(alias)
}

/// Change an identity to what was typed. The account's own address keeps its address, which is
/// the one it signs in as: [`CoreError::OwnAddressStays`] when another was typed for it.
pub fn edit(store: &SqliteStore, id: IdentityId, typed: &Typed) -> Result<Identity, CoreError> {
    let Some(had) = store.identity(id)? else {
        return Err(mail_store::StoreError::NoIdentity(id).into());
    };
    let from = typed.from()?;
    if had.default == IsDefault::Default && !from.email.eq_ignore_ascii_case(&had.from.email) {
        return Err(CoreError::OwnAddressStays);
    }
    let reply_to = typed.reply_to()?;
    store.edit_identity(id, &from, reply_to.as_ref())?;
    mirrored(store, &had.account);
    Ok(Identity {
        from,
        reply_to,
        ..had
    })
}

/// Remove an alias. Drafts written as it go to the account's own address (the store moves
/// them); the account's own address is not removed ([`CoreError::OwnAddressStays`]).
pub fn remove_alias(store: &SqliteStore, id: IdentityId) -> Result<(), CoreError> {
    let Some(had) = store.identity(id)? else {
        return Err(mail_store::StoreError::NoIdentity(id).into());
    };
    if had.default == IsDefault::Default {
        return Err(CoreError::OwnAddressStays);
    }
    store.remove_identity(id)?;
    mirrored(store, &had.account);
    Ok(())
}

/// Write the table's identities into the account's plan, which keeps a copy. Logged rather than
/// failed: the table is what a draft is sent from, and it has already been written; a plan left
/// behind costs the vacation reply an address until the next change, not a message.
fn mirrored(store: &SqliteStore, account: &AccountId) {
    let Some(Some(stored)) = store
        .account(account.clone())
        .or_log("the account whose identities changed could not be read")
    else {
        return;
    };
    let Some(mut plan) = stored
        .plan
        .or_log("the plan of the account whose identities changed could not be read")
    else {
        return;
    };
    let Some(identities) = store
        .identities(account.clone())
        .or_log("the identities just written could not be read back")
    else {
        return;
    };
    if plan.identities == identities {
        return;
    }
    plan.identities = identities;
    store
        .set_account_plan(account.clone(), &plan)
        .or_log("the plan's copy of the identities could not be written");
}
