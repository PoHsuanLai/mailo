//! Setting a Microsoft account that reads over IMAP up again over Microsoft Graph, for an
//! organisation that switched IMAP off.
//!
//! Thunderbird migrates nothing: an account whose server will not take it over IMAP is removed
//! and added again as a Graph one. Here it stays the same account, because everything that would
//! be lost by removing it is safe to keep. The address, the account's id (its keyring key), its
//! identity and signature, its rules and its mail are what they were; only the plan, and where
//! each message is on the server, change. The messages are found again by their `Message-ID` on
//! the first Graph sync ([`mail_store::SqliteStore::forget_addresses`]), as a message a server
//! moved is. Nothing is done unasked: a pass that Microsoft refuses says so and offers this
//! ([`graph_offer`]), and the person chooses it.

use super::{Added, Credentials, Receive, add_receiving, microsoft_account_of};
use crate::error::CoreError;
use crate::sync::report::Offer;
use mail_domain::presets::MicrosoftAccount;
use mail_domain::{AccountPlan, AuthPlan, Incoming, Outgoing};
use mail_store::SqliteStore;
use porter_core::{AccountId, SecretKey, SecretPurpose};
use porter_provider::Issuer;

/// What the window offers beside a pass that ended with the protocol refused: Graph, for a
/// work or school Microsoft account that reads over IMAP, and nothing for any other.
///
/// A personal account is not offered it: its IMAP is not the organisation's to switch off, and
/// what refused it is something Graph would not mend.
pub fn graph_offer(plan: &AccountPlan) -> Option<Offer> {
    let microsoft = matches!(
        plan.auth,
        AuthPlan::OAuth {
            issuer: Issuer::Microsoft,
            ..
        }
    );
    let imap = matches!(plan.incoming, Incoming::Imap { .. });
    let organisation = microsoft_account_of(&plan.address, None) == MicrosoftAccount::Organisation;
    (microsoft && imap && organisation).then_some(Offer::UseGraph)
}

/// Set `account` up again to read and send through Microsoft Graph, and sign it in for Graph.
///
/// The same account afterwards: its plan is rewritten to Graph's, where its messages are on the
/// server and how far each mailbox was fetched are let go of so the first Graph sync starts
/// from nothing, and the Graph token an account sending through Graph kept (minted for
/// `Mail.Send` alone) is forgotten so one for reading too is minted. Then it signs in as adding
/// it does, in the browser unless `credentials` hands over a sign-in already made, and the
/// result says how that went: [`super::Outcome::SignedIn`], or what it still needs.
///
/// Refused, with nothing changed, for an account [`graph_offer`] would not offer it to.
pub async fn switch_to_graph(
    store: &SqliteStore,
    account: AccountId,
    now: chrono::DateTime<chrono::Utc>,
    credentials: Credentials<'_>,
) -> Result<Added, CoreError> {
    let stored = store
        .account(account.clone())?
        .ok_or(CoreError::AccountGone)?;
    let plan = stored.plan?;
    if graph_offer(&plan).is_none() {
        return Err(CoreError::NotMicrosoftImap {
            address: stored.address,
        });
    }
    store.forget_addresses(account.clone())?;
    if plan.outgoing == Outgoing::Graph {
        credentials
            .secrets
            .forget(&SecretKey {
                account,
                purpose: SecretPurpose::OutgoingPassword,
            })
            .await
            .map_err(|e| CoreError::cannot("forget the Graph sign-in for sending", e))?;
    }
    // `--microsoft --receive graph`: named, so an account on IMAP is moved, as asked.
    add_receiving(
        store,
        &stored.address,
        None,
        true,
        false,
        Receive::Graph,
        now,
        credentials,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Outcome;
    use crate::environment::Environment;
    use crate::support_tests::now;
    use mail_domain::{MailboxRef, RemoteRef, SyncCursor};
    use mail_runtime::{AccountSecrets, Arrival, ClientRegistry};
    use mail_store::Store;
    use porter_core::{Credential, SecretText, UnixSeconds};
    use porter_secrets::MemorySecrets;

    const ADDRESS: &str = "someone@contoso.onmicrosoft.com";

    fn token(access: &str) -> Credential {
        Credential::OAuth {
            access: SecretText::new(access.to_owned()),
            refresh: SecretText::new("refresh".to_owned()),
            expires_at: UnixSeconds(now().timestamp() + 3600),
            granted: Default::default(),
        }
    }

    /// An account reading `ADDRESS` over IMAP and sending through Graph, as a work account is
    /// added for a tenant with SMTP off, with one message in its inbox and a cursor there.
    fn on_imap(
        store: &SqliteStore,
        secrets: &MemorySecrets,
    ) -> (AccountId, mail_domain::MessageId) {
        let added = mail_runtime::block_on(super::super::add_receiving(
            store,
            ADDRESS,
            None,
            true,
            true,
            Receive::Imap,
            now(),
            Credentials {
                password: None,
                saved: &ClientRegistry::default(),
                secrets,
                environment: &Environment::default(),
                on_url: &|url| panic!("a sign-in asked for a browser: {url}"),
                signed: None,
            },
        ))
        .unwrap();
        let account = added.account;
        let inbox = MailboxRef {
            account: account.clone(),
            path: "INBOX".to_owned(),
        };
        let raw = "From: ada@example.test\r\nTo: someone@contoso.onmicrosoft.com\r\n\
                   Subject: the plan\r\nDate: Tue, 22 Sep 2026 09:00:00 +0800\r\n\
                   Message-ID: <plan@example.test>\r\n\r\nthe body\r\n";
        mail_runtime::absorb(
            store,
            account.clone(),
            inbox.clone(),
            Some(SyncCursor::Imap {
                uidvalidity: 1,
                uidnext: 8,
                modseq: None,
            }),
            vec![Arrival {
                remote: RemoteRef::Imap {
                    mailbox: "INBOX".to_owned(),
                    uidvalidity: 1,
                    uid: 7,
                },
                raw: raw.as_bytes().to_vec(),
            }],
            false,
            now(),
        )
        .unwrap();
        mail_store::testing::seed_sync_state(
            store,
            account.clone(),
            "INBOX",
            &SyncCursor::Imap {
                uidvalidity: 1,
                uidnext: 8,
                modseq: None,
            },
            None,
        );
        let message = mail_store::testing::message_ids(store)[0];
        mail_runtime::block_on(secrets.put(
            &SecretKey {
                account: account.clone(),
                purpose: SecretPurpose::OutgoingPassword,
            },
            &token("graph-send-only"),
        ))
        .unwrap();
        (account, message)
    }

    /// Switching keeps the account and its mail, rewrites its plan to Graph's, lets go of its
    /// IMAP addresses and cursors and of the send-only Graph token, and asks for a sign-in for
    /// Graph's scopes.
    #[test]
    fn switching_to_graph_keeps_the_account_and_its_mail_and_asks_for_a_sign_in() {
        let store = mail_store::testing::in_memory();
        let secrets = MemorySecrets::default();
        let (account, message) = on_imap(&store, &secrets);
        let inbox = MailboxRef {
            account: account.clone(),
            path: "INBOX".to_owned(),
        };
        assert!(store.cursor(&inbox).unwrap().is_some(), "precondition");
        assert_eq!(store.remote_refs(&inbox).unwrap().len(), 1, "precondition");
        let before = store
            .account(account.clone())
            .unwrap()
            .expect("the account");
        assert_eq!(
            graph_offer(&before.plan.unwrap()),
            Some(Offer::UseGraph),
            "a work account on IMAP is offered Graph"
        );

        let switched = mail_runtime::block_on(switch_to_graph(
            &store,
            account.clone(),
            now(),
            Credentials {
                password: None,
                saved: &ClientRegistry::default(),
                secrets: &secrets,
                environment: &Environment::default(),
                on_url: &|url| panic!("a sign-in asked for a browser: {url}"),
                signed: None,
            },
        ))
        .unwrap();

        assert_eq!(switched.account, account, "the same account");
        assert!(switched.updated);
        let plan = store
            .account(account.clone())
            .unwrap()
            .expect("the account")
            .plan
            .unwrap();
        assert_eq!(plan.incoming, Incoming::Graph);
        assert_eq!(plan.outgoing, Outgoing::Graph);
        assert_eq!(plan.address, ADDRESS);
        assert_eq!(plan.identities.len(), 1, "the identity is kept");
        assert_eq!(graph_offer(&plan), None, "nothing more to offer");

        assert_eq!(
            store.cursor(&inbox).unwrap(),
            None,
            "the IMAP cursor is gone"
        );
        assert!(
            store.remote_refs(&inbox).unwrap().is_empty(),
            "the IMAP addresses are gone"
        );
        assert_eq!(
            store.message(message).unwrap().id,
            message,
            "the mail is kept, to be found again by its Message-ID"
        );
        assert!(
            mail_runtime::block_on(secrets.get(&SecretKey {
                account: account.clone(),
                purpose: SecretPurpose::OutgoingPassword,
            }))
            .is_err(),
            "the send-only Graph token is forgotten"
        );

        // A sign-in is asked for, for Graph's scopes: with no client at hand (the registry handed
        // in is empty, on purpose), it says so.
        let Outcome::NeedsClientId { issuer, scopes, .. } = switched.outcome else {
            panic!("{:?}", switched.outcome)
        };
        assert_eq!(issuer, Issuer::Microsoft);
        let mut scopes: Vec<&str> = scopes.iter().map(String::as_str).collect();
        scopes.sort_unstable();
        let mut graph = mail_domain::presets::GRAPH_SCOPES.to_vec();
        graph.sort_unstable();
        assert_eq!(scopes, graph);
    }

    /// Only a work or school Microsoft account on IMAP is moved; anything else is refused and
    /// left as it was.
    #[test]
    fn only_a_work_account_on_imap_is_offered_graph_or_switched() {
        let microsoft = |address: &str| mail_domain::presets::microsoft_preset(address, now()).plan;
        assert_eq!(
            graph_offer(&microsoft("me@contoso.example")),
            Some(Offer::UseGraph)
        );
        assert_eq!(graph_offer(&microsoft("me@outlook.com")), None, "personal");
        let graph = mail_domain::presets::receive_through_graph(
            mail_domain::presets::microsoft_preset("me@contoso.example", now()),
        );
        assert_eq!(graph_offer(&graph.plan), None, "already on Graph");
        let gmail =
            mail_domain::presets::preset_for_issuer(Issuer::Google, "me@gmail.com", now()).unwrap();
        assert_eq!(graph_offer(&gmail.plan), None, "not Microsoft");

        let store = mail_store::testing::in_memory();
        let id = mail_domain::id::new_account_id();
        mail_store::testing::seed_account_plan(
            &store,
            id.clone(),
            "me@gmail.com",
            &gmail.plan,
            None,
        );
        let refused = mail_runtime::block_on(switch_to_graph(
            &store,
            id.clone(),
            now(),
            Credentials {
                password: None,
                saved: &ClientRegistry::default(),
                secrets: &MemorySecrets::default(),
                environment: &Environment::default(),
                on_url: &|url| panic!("a sign-in asked for a browser: {url}"),
                signed: None,
            },
        ));
        assert!(
            matches!(refused, Err(CoreError::NotMicrosoftImap { .. })),
            "{refused:?}"
        );
        let kept = store
            .account(id)
            .unwrap()
            .expect("the account")
            .plan
            .unwrap();
        assert_eq!(kept, gmail.plan, "nothing changed");
    }
}
