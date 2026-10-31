use super::{Fit, HEADERS, Measure, PART_HEADERS, Picture, Said, SendLimit, Weight, fit};
use mail_logic::picture::PictureSize;

const MB: u64 = 1024 * 1024;

fn limit(octets: u64, measure: Measure) -> SendLimit {
    SendLimit {
        octets,
        measure,
        said: Said::Server,
    }
}

/// A message of no text with `files`, each the same at every size but a picture's.
fn carrying(files: &[([u64; 4], Picture)]) -> Weight {
    Weight::of(0, files)
}

#[test]
fn a_message_weighs_its_text_and_files_as_they_go_and_as_they_were() {
    let weight = Weight::of(
        100,
        &[([57; 4], Picture::No), ([114, 228, 342, 456], Picture::Yes)],
    );
    let wire = |picture: u64| {
        HEADERS
            + mail_mime::base64_len(100)
            + 2 * PART_HEADERS
            + 78
            + mail_mime::base64_len(picture)
    };
    let cases = [
        (PictureSize::Small, 114),
        (PictureSize::Medium, 228),
        (PictureSize::Large, 342),
        (PictureSize::Actual, 456),
    ];
    for (size, picture) in cases {
        assert_eq!(
            weight.against(size, limit(MB, Measure::Content)),
            100 + 57 + picture,
            "{size:?} as written"
        );
        assert_eq!(
            weight.against(size, limit(MB, Measure::Wire)),
            wire(picture),
            "{size:?} as it goes"
        );
    }
    assert_eq!(weight.pictures(), 1);
}

#[test]
fn a_message_over_its_limit_says_so_and_names_the_largest_picture_size_that_fits() {
    let photos = || carrying(&[([2 * MB, 5 * MB, 12 * MB, 30 * MB], Picture::Yes)]);
    let files = || carrying(&[([30 * MB; 4], Picture::No)]);
    let content = |octets| limit(octets, Measure::Content);
    let over = |size, limit, smaller| Fit::Over {
        size,
        limit,
        smaller,
    };
    let cases = [
        (
            "within",
            photos(),
            PictureSize::Actual,
            content(40 * MB),
            Fit::Fits,
        ),
        (
            "exactly at the limit fits",
            photos(),
            PictureSize::Actual,
            content(30 * MB),
            Fit::Fits,
        ),
        (
            "over, and large fits",
            photos(),
            PictureSize::Actual,
            content(25 * MB),
            over(30 * MB, content(25 * MB), Some(PictureSize::Large)),
        ),
        (
            "over, and only small fits",
            photos(),
            PictureSize::Actual,
            content(3 * MB),
            over(30 * MB, content(3 * MB), Some(PictureSize::Small)),
        ),
        (
            "over at medium, and small fits",
            photos(),
            PictureSize::Medium,
            content(3 * MB),
            over(5 * MB, content(3 * MB), Some(PictureSize::Small)),
        ),
        (
            "over at small, and nothing smaller is offered",
            photos(),
            PictureSize::Small,
            content(MB),
            over(2 * MB, content(MB), None),
        ),
        (
            "over with no pictures, and nothing is offered",
            files(),
            PictureSize::Actual,
            content(25 * MB),
            over(30 * MB, content(25 * MB), None),
        ),
    ];
    for (case, weight, chosen, limit, expected) in cases {
        assert_eq!(fit(&weight, chosen, limit), expected, "{case}");
    }
}

#[test]
fn a_limit_counts_the_message_its_own_way() {
    // 20 MB attached is about 27 MB in base64: within a limit of what was attached, over one
    // of what goes.
    let weight = carrying(&[([20 * MB; 4], Picture::No)]);
    assert_eq!(
        fit(
            &weight,
            PictureSize::Actual,
            limit(25 * MB, Measure::Content)
        ),
        Fit::Fits
    );
    assert!(matches!(
        fit(&weight, PictureSize::Actual, limit(25 * MB, Measure::Wire)),
        Fit::Over { size, .. } if size > 27 * MB
    ));
}

#[test]
fn an_accounts_limit_is_what_its_server_said_or_graphs_or_the_usual() {
    use mail_domain::Outgoing;
    use mail_domain::presets::{self, Manual};

    let store = mail_store::testing::in_memory();
    let now = mail_domain::testing::now();
    let preset = presets::manual(
        "ada@example.test",
        &Manual {
            imap_host: "imap.example.test".to_owned(),
            imap_port: 993,
            smtp_host: "smtp.example.test".to_owned(),
            smtp_port: 465,
            login: None,
        },
        now,
    );
    let seed = |address: &str, outgoing: Outgoing, said: Option<u64>| {
        let id = mail_domain::id::new_account_id();
        let plan = mail_domain::AccountPlan {
            outgoing,
            ..preset.plan.clone()
        };
        mail_store::testing::seed_account_plan(&store, id.clone(), address, &plan, None);
        mail_store::testing::seed_caps(&store, id.clone(), &preset.expected_caps, now)
            .unwrap_or_else(|why| panic!("caps: {why}"));
        store
            .put_send_limit(id.clone(), said)
            .unwrap_or_else(|why| panic!("limit: {why}"));
        id
    };
    let smtp = || Outgoing::Smtp {
        host: "smtp.example.test".to_owned(),
        port: 465,
        tls: mail_domain::Tls::Implicit,
    };
    let server = |octets| SendLimit {
        octets,
        measure: Measure::Wire,
        said: Said::Server,
    };
    let graph = SendLimit {
        octets: mail_runtime::graph::MAX_MESSAGE,
        measure: Measure::Content,
        said: Said::Graph,
    };
    let cases = [
        (
            "smtp that said SIZE",
            seed("a@x.test", smtp(), Some(10 * MB)),
            server(10 * MB),
        ),
        (
            "smtp not yet heard",
            seed("b@x.test", smtp(), None),
            SendLimit::USUAL,
        ),
        (
            "jmap that said",
            seed("c@x.test", Outgoing::Jmap, Some(50 * MB)),
            server(50 * MB),
        ),
        // Graph says nothing, and what a stray number says is not what Graph takes.
        ("graph", seed("d@x.test", Outgoing::Graph, Some(MB)), graph),
        (
            "local",
            seed("e@x.test", Outgoing::Nowhere, None),
            SendLimit::USUAL,
        ),
        (
            "an account not here",
            mail_domain::id::new_account_id(),
            SendLimit::USUAL,
        ),
    ];
    for (case, account, expected) in cases {
        assert_eq!(super::send_limit(&store, &account), expected, "{case}");
    }
}

/// Graph's `sendMail` cannot ask for delivery reports, so the composer offers the choice for every
/// account but a Graph one.
#[test]
fn delivery_reports_are_offered_for_every_account_but_graph() {
    use mail_domain::Outgoing;
    use mail_domain::presets::{self, Manual};

    let store = mail_store::testing::in_memory();
    let now = mail_domain::testing::now();
    let preset = presets::manual(
        "ada@example.test",
        &Manual {
            imap_host: "imap.example.test".to_owned(),
            imap_port: 993,
            smtp_host: "smtp.example.test".to_owned(),
            smtp_port: 465,
            login: None,
        },
        now,
    );
    let seed = |address: &str, outgoing: Outgoing| {
        let id = mail_domain::id::new_account_id();
        let plan = mail_domain::AccountPlan {
            outgoing,
            ..preset.plan.clone()
        };
        mail_store::testing::seed_account_plan(&store, id.clone(), address, &plan, None);
        id
    };
    let graph = seed("g@x.test", Outgoing::Graph);
    let jmap = seed("j@x.test", Outgoing::Jmap);
    let smtp = seed("s@x.test", preset.plan.outgoing.clone());
    assert!(!super::can_ask_delivery_report(&store, &graph));
    assert!(super::can_ask_delivery_report(&store, &jmap));
    assert!(super::can_ask_delivery_report(&store, &smtp));
}

/// The strip says a picture goes without its location only where one is made smaller: one that
/// already fits goes as it was, so the size it is sent at changes nothing.
#[test]
fn a_size_resizes_only_where_a_picture_comes_out_smaller() {
    // One photo that shrinks at every size but Large, which is as large as it is; and a file.
    let photo = [10, 20, 40, 40];
    let weight = carrying(&[(photo, Picture::Yes), ([5; 4], Picture::No)]);
    assert!(weight.resizes_at(PictureSize::Small));
    assert!(weight.resizes_at(PictureSize::Medium));
    assert!(!weight.resizes_at(PictureSize::Large));
    assert!(!weight.resizes_at(PictureSize::Actual));
    // A picture that already fits comes out the same at every size.
    let fits = carrying(&[([30; 4], Picture::Yes)]);
    for size in PictureSize::ALL {
        assert!(!fits.resizes_at(size), "{size:?}");
    }
}
