//! The operators added after the first vocabulary: `cc:`, sizes, relative dates, `account:` and
//! `OR`. The rest are tested from the window's side, in `mail-app/tests/app/searching.rs`.

use super::*;
fn account(n: u128) -> AccountId {
    mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(n))
}

fn accounts() -> Vec<(String, AccountId)> {
    vec![
        ("work@example.com".to_owned(), account(1)),
        ("me@home.example".to_owned(), account(2)),
        ("me@other.example".to_owned(), account(3)),
    ]
}

fn parse(input: &str) -> Filter {
    let accounts = accounts();
    let known = Known {
        labels: &[],
        accounts: &accounts,
    };
    parse_with(input, &Utc, &known)
}

fn text(word: &str) -> Filter {
    Filter::Text(TextMatch::Contains(word.to_owned()))
}

fn from(word: &str) -> Filter {
    Filter::From(TextMatch::Contains(word.to_owned()))
}

fn since(days: u32) -> Filter {
    Filter::NewerThan(days)
}

#[test]
fn each_new_operator_builds_its_filter() {
    const M: u64 = 1024 * 1024;
    let cases: Vec<(&str, &str, Filter)> = vec![
        (
            "cc: is the Cc line alone",
            "cc:bob",
            Filter::Cc(TextMatch::Contains("bob".to_owned())),
        ),
        (
            "to: is everyone addressed",
            "to:bob",
            Filter::To(TextMatch::Contains("bob".to_owned())),
        ),
        ("larger: in mebibytes", "larger:5M", Filter::Larger(5 * M)),
        (
            "smaller: in kibibytes",
            "smaller:100K",
            Filter::Smaller(100 * 1024),
        ),
        (
            "size:> with a fraction",
            "size:>1.5M",
            Filter::Larger(M + M / 2),
        ),
        ("size:< in bytes", "size:<500", Filter::Smaller(500)),
        ("a size that is not one", "larger:big", text("larger:big")),
        ("size: with no comparison", "size:5M", text("size:5M")),
        ("newer: days", "newer:7d", since(7)),
        ("newer: weeks", "newer:2w", since(14)),
        ("newer: months", "newer:3m", since(90)),
        ("newer: a bare count is days", "newer:10", since(10)),
        (
            "newer_than: as Gmail spells it",
            "newer_than:1y",
            since(365),
        ),
        ("older:", "older:2w", Filter::OlderThan(14)),
        ("no span at all", "newer:0d", text("newer:0d")),
        (
            "account: by address",
            "account:work@example.com",
            Filter::Account(account(1)),
        ),
        (
            "account: by the part before @",
            "account:WORK",
            Filter::Account(account(1)),
        ),
        (
            "account: two share the name",
            "account:me",
            text("account:me"),
        ),
        ("account: nobody", "account:nobody", text("account:nobody")),
        (
            "OR between operators",
            "from:ada OR from:bob",
            Filter::Or(vec![from("ada"), from("bob")]),
        ),
        (
            "| between words",
            "lunch | dinner",
            Filter::Or(vec![text("lunch"), text("dinner")]),
        ),
        (
            "OR binds tighter than the words around it",
            "ada lunch OR dinner",
            Filter::And(vec![
                Filter::Or(vec![text("lunch"), text("dinner")]),
                text("ada"),
            ]),
        ),
        (
            "three ways",
            "from:a OR from:b OR from:c",
            Filter::Or(vec![from("a"), from("b"), from("c")]),
        ),
        (
            "a negated side",
            "-from:ada OR from:bob",
            Filter::Or(vec![Filter::Not(Box::new(from("ada"))), from("bob")]),
        ),
        ("OR with nothing before it", "OR lunch", text("lunch")),
        ("OR with nothing after it", "lunch OR", text("lunch")),
        (
            "lower-case or is a word",
            "lunch or dinner",
            text("lunch or dinner"),
        ),
        (
            "a quoted OR is a phrase",
            "\"OR\"",
            Filter::Text(TextMatch::Exact("OR".to_owned())),
        ),
    ];
    for (case, typed, want) in cases {
        assert_eq!(parse(typed), want, "{case}: {typed:?}");
    }
}

/// With no accounts to hand — a resolver of labels alone — `account:` is text, as any term
/// nobody recognises is.
#[test]
fn without_accounts_an_account_term_is_text() {
    for typed in ["account:work", "account:me@home.example"] {
        assert_eq!(parse_with(typed, &Utc, &unnamed), text(typed), "{typed}");
    }
}

/// A size written back reads as the same size.
#[test]
fn a_size_reads_back_as_written() {
    for bytes in [0, 1, 1023, 1024, 1536, 5 * 1024 * 1024, 7_000_001] {
        let words = mail_domain::filter::Words {
            label: &|_| String::new(),
            folder: &|path| path.to_owned(),
            account: "",
            snooze_due: "",
            any_date: "",
        };
        for filter in [Filter::Larger(bytes), Filter::Smaller(bytes)] {
            let spelled = filter.spelled(&words);
            assert_eq!(parse(&spelled), filter, "{spelled}");
        }
    }
}

/// `cc:` and `is:vip` written back read as the same clause.
#[test]
fn a_copy_and_a_vip_read_back_as_written() {
    let words = mail_domain::filter::Words {
        label: &|_| String::new(),
        folder: &|path| path.to_owned(),
        account: "",
        snooze_due: "",
        any_date: "",
    };
    for filter in [
        Filter::Cc(TextMatch::Contains("bob".to_owned())),
        Filter::Cc(TextMatch::Contains("bob smith".to_owned())),
        Filter::Vip,
    ] {
        let spelled = filter.spelled(&words);
        assert_eq!(parse(&spelled), filter, "{spelled}");
    }
}

/// `is:vip`: a clause that follows the VIP list.
mod vip {
    use super::*;
    use mail_domain::ReadState;

    #[test]
    fn is_vip_is_the_clause_that_follows_the_list() {
        // (case, typed, the filter): the list is never read here, so none of these keeps anyone.
        let cases: Vec<(&str, &str, Filter)> = vec![
            ("is:vip", "is:vip", Filter::Vip),
            ("its plural, any case", "is:VIPs", Filter::Vip),
            ("negated", "-is:vip", Filter::Not(Box::new(Filter::Vip))),
            (
                "beside another term",
                "is:vip is:unread",
                Filter::And(vec![Filter::Vip, Filter::Read(ReadState::Unread)]),
            ),
            (
                "is:vipx is a word",
                "is:vipx",
                Filter::Text(TextMatch::Contains("is:vipx".to_owned())),
            ),
        ];
        for (case, typed, want) in cases {
            assert_eq!(parse(typed), want, "{case}: {typed:?}");
        }
    }
}

/// An either-or of single terms, and relative dates, written back read as the same filter: what
/// a rule kept as `a OR b` opens on again.
#[test]
fn an_either_or_reads_back_as_written() {
    let words = mail_domain::filter::Words {
        label: &|_| String::new(),
        folder: &|path| path.to_owned(),
        account: "",
        snooze_due: "",
        any_date: "",
    };
    let cases = [
        Filter::Or(vec![from("ada"), from("bob")]),
        Filter::Or(vec![
            Filter::Larger(5 * 1024 * 1024),
            Filter::Subject(TextMatch::Contains("lunch on friday".to_owned())),
            Filter::OlderThan(30),
        ]),
        Filter::NewerThan(7),
    ];
    for filter in cases {
        let spelled = filter.spelled(&words);
        assert_eq!(parse(&spelled), filter, "{spelled}");
    }
}

#[test]
fn a_filter_is_spelled_in_the_words_it_was_written_in() {
    let index: Vec<(String, LabelId)> = vec![("Money".into(), LabelId::generate())];
    let label_id = index[0].1;
    let filter = parse_with(
        "from:bank.example -is:read label:money \"due date\"",
        &Utc,
        &named(&index),
    );
    let name = |id: LabelId| {
        if id == label_id {
            "Money".to_owned()
        } else {
            "?".to_owned()
        }
    };
    assert_eq!(
        condition(&filter, &name),
        "from:bank.example -is:read label:Money \"due date\""
    );
}
