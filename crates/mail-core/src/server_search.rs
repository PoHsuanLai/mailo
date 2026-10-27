//! Searching the server from a search typed here: which accounts can, whether it runs by itself,
//! and the typed line as the filter a server is asked for.
//!
//! A search runs over the store. When it is shown, the list's end offers to search each account
//! in view on its server (`ui::server_search`), which finds what the store never held. It is
//! asked, not automatic, unless the user turned "Search the server automatically" on: a server
//! search is a connection and a query against someone else's machine, and a keystroke should
//! not make one. The switch is the window's (`search.server_automatically` in mail-app's
//! `mailo/settings.toml`; `server-search.json` before it), and off unless turned on.
//!
//! What a server is asked is the line exactly as the store is asked it, parsed by the same
//! [`crate::query::parse_with`], except that `label:` resolves against the one account being
//! searched — a label is a name on one server — and a `re:/…/` pattern, which no server
//! matches, is said rather than dropped. Each protocol's translation is `mail_proto::search`'s;
//! the engines are `mail_runtime`'s ([`crate::SyncOps::search_server`]).

use crate::config::read_json;
use crate::error::CoreError;
use mail_domain::{AccountPlan, Filter, Incoming, LabelId};
use mail_runtime::{Searched, Unsaid};
use porter_core::AccountId;
use std::path::Path;

/// Whether a search shown in the list is asked of the server without the button being pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Automatic {
    On,
    /// Only when "Search … on the server" is pressed.
    #[default]
    Off,
}

const FILE_NAME: &str = "server-search.json";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    #[serde(default)]
    automatic: Automatic,
}

/// What `server-search.json` said, or off when there is none or it cannot be read. Never
/// written: mail-app keeps the switch (`search.server_automatically`) and read this once.
pub fn load(dir: &Path) -> Automatic {
    read_json::<Stored>(dir, FILE_NAME).automatic
}

/// Whether an account has a server to search: IMAP, JMAP, or Microsoft Graph. POP3 has one
/// mailbox and no search, and local folders have no server.
pub fn searchable(plan: &AccountPlan) -> bool {
    matches!(
        plan.incoming,
        Incoming::Imap { .. } | Incoming::Jmap { .. } | Incoming::Graph
    )
}

/// The search line `input` as a server is asked it, on an account whose labels are `labels`.
///
/// `Err` when a part of it no server can match: a `re:/…/` pattern, which is matched here over
/// what the store holds. An invalid pattern is that too: it matches nothing anywhere.
///
/// Parsed with the names the store parses with ([`crate::query::Known`]): `label:` against this
/// account's `labels`, and `account:` against every account's address (`accounts`), which a
/// server is told of as it is any account's clause. `is:vip` is asked as the VIPs' addresses
/// (`vips`), each the address alone: the list is the store's, which a server cannot read.
pub fn filter_of(
    input: &str,
    labels: &[(String, LabelId)],
    accounts: &[(String, AccountId)],
    vips: &crate::vip::Vips,
) -> Result<Filter, Unsaid> {
    let extracted = crate::search::extract(input).map_err(|why| Unsaid(vec![why.to_string()]))?;
    if let Some(pattern) = &extracted.regex {
        return Err(Unsaid(vec![format!(
            "re:/{}/ (a pattern is matched on this computer, not by a server)",
            pattern.as_str()
        )]));
    }
    let known = crate::query::Known { labels, accounts };
    Ok(with_vips(
        crate::query::parse_with(&extracted.rest, &chrono::Local, &known),
        vips,
    ))
}

/// `filter` with each `is:vip` clause in it said as the addresses of `vips`, for a server that
/// has no such list.
fn with_vips(filter: Filter, vips: &crate::vip::Vips) -> Filter {
    match filter {
        // Nobody on the list is a clause that finds nothing, which every translation has a
        // word for, where an either-or of no clauses it has not.
        Filter::Vip => match vips.addresses() {
            [] => Filter::Nothing,
            [one] => Filter::FromAddress(one.clone()),
            many => Filter::Or(many.iter().cloned().map(Filter::FromAddress).collect()),
        },
        Filter::And(parts) => Filter::And(parts.into_iter().map(|f| with_vips(f, vips)).collect()),
        Filter::Or(parts) => Filter::Or(parts.into_iter().map(|f| with_vips(f, vips)).collect()),
        Filter::Not(inner) => Filter::Not(Box::new(with_vips(*inner, vips))),
        other => other,
    }
}

/// Whether the line `input` asks for something only a server knows: a Gmail category, which a
/// message held here never says. Searched on this computer alone it lists nothing (and `-category:`
/// everything), so such a line is asked of the server without waiting to be told to, whatever the
/// automatic setting says; and where the server cannot answer either, the answer says why.
pub fn needs_server(input: &str) -> bool {
    fn names_category(filter: &Filter) -> bool {
        match filter {
            Filter::Category(_) => true,
            Filter::And(parts) | Filter::Or(parts) => parts.iter().any(names_category),
            Filter::Not(part) => names_category(part),
            _ => false,
        }
    }
    filter_of(input, &[], &[], &crate::vip::Vips::default())
        .is_ok_and(|filter| names_category(&filter))
}

/// Search `account`'s server for the line `input`, and keep what it finds.
pub async fn search(
    mail: &crate::Mail,
    account: AccountId,
    input: &str,
) -> Result<Searched, CoreError> {
    let named: Vec<(String, LabelId)> = mail
        .store()
        .labels(account.clone())?
        .into_iter()
        .map(|l| (l.name, l.id))
        .collect();
    let accounts = crate::query::known_accounts(mail.store());
    let filter = match filter_of(
        input,
        &named,
        &accounts,
        &crate::vip::read(mail.store().as_ref()),
    ) {
        Ok(filter) => filter,
        Err(unsaid) => return Ok(Searched::Unsaid(unsaid)),
    };
    let labels: Vec<(LabelId, String)> = named.into_iter().map(|(n, id)| (id, n)).collect();
    mail.sync().search_server(account, &filter, &labels).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::TextMatch;

    #[test]
    fn a_category_is_the_servers_to_say_and_nothing_else_is() {
        for (input, want) in [
            ("category:promotions", true),
            ("from:ada -category:social", true),
            ("from:ada category:forums OR subject:lunch", true),
            ("from:ada", false),
            ("promotions", false),
        ] {
            assert_eq!(needs_server(input), want, "{input}");
        }
    }

    #[test]
    fn off_until_turned_on() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Automatic::Off);
        std::fs::write(dir.path().join(FILE_NAME), r#"{"automatic":"on"}"#).unwrap();
        assert_eq!(load(dir.path()), Automatic::On);
    }

    #[test]
    fn the_line_is_parsed_as_the_store_parses_it_and_a_pattern_is_said() {
        let travel = LabelId::generate();
        let labels = [("travel".to_owned(), travel)];
        let cases: Vec<(&str, Result<Filter, ()>)> = vec![
            (
                "from:ada",
                Ok(Filter::From(TextMatch::Contains("ada".to_owned()))),
            ),
            ("label:travel", Ok(Filter::HasLabel(travel))),
            (
                "label:work",
                Ok(Filter::Text(TextMatch::Contains("label:work".to_owned()))),
            ),
            ("re:/lun.h/ friday", Err(())),
        ];
        for (input, expected) in cases {
            assert_eq!(
                filter_of(input, &labels, &[], &crate::vip::Vips::default()).map_err(|_| ()),
                expected,
                "{input}"
            );
        }
        let said = filter_of("re:/lun.h/", &labels, &[], &crate::vip::Vips::default()).unwrap_err();
        assert_eq!(
            said.0,
            vec!["re:/lun.h/ (a pattern is matched on this computer, not by a server)".to_owned()]
        );
    }

    #[test]
    fn an_account_and_the_vips_are_said_as_the_store_hears_them() {
        let work = mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(1));
        let accounts = [("work@example.test".to_owned(), work.clone())];
        let some = crate::vip::Vips::new(["Dana@example.test", "bob@example.test"]);
        let said =
            |input: &str, vips: &crate::vip::Vips| filter_of(input, &[], &accounts, vips).unwrap();
        assert_eq!(
            said("account:work", &some),
            Filter::Account(work),
            "an account is the account, not the text account:work"
        );
        assert_eq!(
            said("is:vip", &some),
            Filter::Or(vec![
                Filter::FromAddress("bob@example.test".to_owned()),
                Filter::FromAddress("dana@example.test".to_owned()),
            ]),
            "the address alone, each, as the list keeps them"
        );
        assert_eq!(
            said("-is:vip", &crate::vip::Vips::new(["bob@example.test"])),
            Filter::Not(Box::new(Filter::FromAddress("bob@example.test".to_owned())))
        );
        assert_eq!(
            said("is:vip", &crate::vip::Vips::default()),
            Filter::Nothing,
            "nobody on the list finds nothing, not an either-or of none"
        );
    }
}
