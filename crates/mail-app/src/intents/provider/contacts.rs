//! The person's address book: finding someone in it, and who an entity of it names.
//!
//! A contact is keyed by its address, as the store keys it. What the book holds is the person's
//! own (names they gave, addresses they wrote to), so it is labelled trusted, unlike the mail it
//! was learned from.

use super::{Provider, outcome};
use crate::intents::APP;
use crate::intents::wire::{AppRefusal, EntityId, Invocation, Label, Labelled, Outcome, Output};
use mail_domain::Address;
use mail_store::Store;

/// The most a contact search answers with.
const LIMIT: usize = 25;

fn contact_id(address: &str) -> EntityId {
    EntityId {
        app: APP.to_owned(),
        kind: "mail.contact".to_owned(),
        key: address.to_owned(),
    }
}

impl Provider {
    /// `mail.contact.search`: the people whose name or address the query matches, best first.
    pub(super) fn contacts(&self, invocation: &Invocation) -> Result<Outcome, AppRefusal> {
        let query = invocation.text("query").ok_or(AppRefusal::NeedsParam {
            param: "query".to_owned(),
            options: Vec::new(),
        })?;
        let found = self
            .store
            .contacts_matching(query, LIMIT)
            .map_err(|why| AppRefusal::Failed(why.to_string()))?;
        let said = match found.len() {
            0 => "Nobody found".to_owned(),
            1 => "Found 1 contact".to_owned(),
            n => format!("Found {n} contacts"),
        };
        let ids = Labelled {
            value: Output::Entities(found.iter().map(|c| contact_id(&c.address)).collect()),
            label: Label::contacts(&invocation.space),
        };
        Ok(outcome(Some(said), None, Some(ids)))
    }

    /// The address a `mail.contact` entity names, or why not.
    pub(super) fn contact_address(&self, id: &EntityId) -> Result<Address, AppRefusal> {
        if id.app != APP || id.kind != "mail.contact" {
            return Err(AppRefusal::NotFound(id.clone()));
        }
        let contact = self
            .store
            .contact(&id.key)
            .ok()
            .flatten()
            .ok_or_else(|| AppRefusal::NotFound(id.clone()))?;
        Ok(Address {
            name: contact.name,
            email: contact.address,
        })
    }
}
