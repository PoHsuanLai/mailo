use super::*;
use crate::ui::space::Scope;
use mail_domain::id::account_id_from_uuid;

fn acct_a() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d1"))
}
fn acct_b() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d2"))
}
fn acct_c() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d3"))
}
/// Keeps its mail here, so it has no link.
fn acct_local() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d4"))
}

/// What is pressed, the Space's scope, and the accounts covered.
type Case<'a> = (&'a str, Option<AccountId>, Scope, &'a [AccountId]);

fn of(ids: &[AccountId]) -> Scope {
    Scope::Accounts(ids.to_vec())
}

#[test]
fn what_a_press_of_sync_covers() {
    let with_links: BTreeSet<AccountId> = [acct_a(), acct_b(), acct_c()].into();
    let cases: &[Case] = &[
        (
            "every account when nothing narrows it",
            None,
            Scope::All,
            &[acct_a(), acct_b(), acct_c()],
        ),
        (
            "a pressed tile is that account",
            Some(acct_b()),
            Scope::All,
            &[acct_b()],
        ),
        (
            "the tile beats the Space",
            Some(acct_b()),
            of(&[acct_a(), acct_c()]),
            &[acct_b()],
        ),
        (
            "a Space is its accounts",
            None,
            of(&[acct_a(), acct_c()]),
            &[acct_a(), acct_c()],
        ),
        ("a Space of one", None, of(&[acct_c()]), &[acct_c()]),
        ("a Space of no accounts covers none", None, of(&[]), &[]),
        (
            "the local account has nothing to fetch",
            Some(acct_local()),
            Scope::All,
            &[],
        ),
        ("a Space of only local mail", None, of(&[acct_local()]), &[]),
        (
            "local mail beside a server",
            None,
            of(&[acct_local(), acct_a()]),
            &[acct_a()],
        ),
    ];
    for (name, pressed, scope, expected) in cases {
        assert_eq!(
            in_scope(&with_links, pressed.clone(), scope),
            *expected,
            "{name}"
        );
    }
}

#[test]
fn the_window_asks_its_shell() {
    let with_links: BTreeSet<AccountId> = [acct_a(), acct_b()].into();
    let shell = Shell {
        scope: of(&[acct_b()]),
        ..Shell::default()
    };
    assert_eq!(in_view(&with_links, &shell), [acct_b()]);
    let pressed = Shell {
        account: Some(acct_a()),
        scope: of(&[acct_b()]),
        ..Shell::default()
    };
    assert_eq!(in_view(&with_links, &pressed), [acct_a()]);
}
