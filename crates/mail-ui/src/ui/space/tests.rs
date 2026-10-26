//! mailo's part of the Spaces: what a Space holds that is mail's, how the window boots them, and
//! that every `spaces.json` mailo ever wrote still reads as it did. How quire's kit reads a file
//! field by field is quire's to test; these are mailo's files through it.

use super::{Mail, Pinned, Recall, Scope, SpaceId, Spaces, boot, first_run, load};
use crate::ui::appearance::{Legacy, WindowDirs};
use ds::prelude::{SpaceLook, Theme};
use ds::style::space::palette::Dot;
use ds::style::space::presets::PRESETS;
use mail_core::profile::ProfileId;
use mail_domain::ThreadId;
use mail_domain::id::account_id_from_uuid;
use porter_core::AccountId;
use uuid::Uuid;

fn account(n: u128) -> AccountId {
    account_id_from_uuid(Uuid::from_u128(n))
}

/// A config and a state directory, in one scratch directory.
fn dirs() -> (tempfile::TempDir, WindowDirs) {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let dirs = WindowDirs {
        config: dir.path().join("config"),
        state: dir.path().join("state"),
    };
    std::fs::create_dir_all(&dirs.config).unwrap_or_else(|e| panic!("{e}"));
    (dir, dirs)
}

fn write(dirs: &WindowDirs, text: &str) {
    std::fs::write(dirs.config.join("spaces.json"), text).unwrap_or_else(|e| panic!("{e}"));
}

/// Each Space's name, look and mail, in order.
fn shown(spaces: &Spaces) -> Vec<(String, SpaceLook, Mail)> {
    spaces
        .list()
        .iter()
        .map(|space| {
            (
                space.name.clone(),
                space.look.clone(),
                space.payload.clone(),
            )
        })
        .collect()
}

/// A `spaces.json` exactly as mailo wrote it before the look was quire's: flat look fields, the
/// card accent spelled `hint`, a `motion` nobody reads, each Space over every account. It must
/// still load to the same Spaces, each naming the accounts the store has, and the file written
/// back must keep the flat shape and read back unchanged.
const BEFORE_QUIRE: &str = r#"{"spaces":[{"name":"Work","dots":[{"hue":268.0,"chroma":0.72},{"hue":318.0,"chroma":0.55}],"grain":35,"theme":"dark","motion":"calm","card_accent":"hint","scope":{"kind":"all"},"pins":[{"kind":"person","name":"Dana","email":"dana@example.com"}],"colors":{}},{"name":"Home","dots":[{"hue":152.0,"chroma":0.62}],"grain":55,"theme":"system","motion":"standard","card_accent":"postmark","scope":{"kind":"all"},"pins":[],"colors":{}}],"current":1,"recall":{}}
"#;

#[test]
fn a_spaces_file_from_before_quire_still_loads_and_round_trips() {
    let (_dir, dirs) = dirs();
    write(&dirs, BEFORE_QUIRE);
    let read = boot(Some(&dirs), &[account(1)], &Legacy::default());
    let want = vec![
        (
            "Work".to_owned(),
            SpaceLook {
                grain: ds::prelude::Grain(35),
                dots: vec![
                    Dot {
                        hue: 268.0,
                        chroma: 0.72,
                    },
                    Dot {
                        hue: 318.0,
                        chroma: 0.55,
                    },
                ],
                theme: Theme::Dark,
            },
            Mail {
                accounts: vec![account(1)],
                pins: vec![Pinned::Person {
                    name: "Dana".to_owned(),
                    email: "dana@example.com".to_owned(),
                }],
                profile: ProfileId::default(),
            },
        ),
        (
            "Home".to_owned(),
            SpaceLook {
                grain: ds::prelude::Grain(55),
                dots: vec![Dot {
                    hue: 152.0,
                    chroma: 0.62,
                }],
                theme: Theme::System,
            },
            Mail::of(vec![account(1)]),
        ),
    ];
    assert_eq!(shown(&read), want);
    assert_eq!(read.current().name, "Home", "`current` is a position");

    // Booting wrote the file it brought up to date.
    let written: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dirs.config.join("spaces.json")).unwrap_or_else(|e| panic!("{e}")),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let work = &written["spaces"][0];
    // Flat, as before: neither the look nor mail's own fields are nested under a key.
    assert!(work.get("look").is_none(), "{work}");
    assert!(work.get("payload").is_none(), "{work}");
    assert_eq!(work["grain"], 35, "{work}");
    assert!(work.get("motion").is_none(), "{work}");
    // The retired card accent is read and not written back.
    assert!(work.get("card_accent").is_none(), "{work}");
    assert!(work.get("scope").is_none(), "{work}");
    assert_eq!(work["accounts"], serde_json::json!([account(1)]), "{work}");
    assert_eq!(work["pins"][0]["email"], "dana@example.com", "{work}");
    assert_eq!(shown(&load(&dirs)), want, "after saving");
    assert_eq!(load(&dirs).current().name, "Home", "after saving");
}

/// The shape of the file mailo wrote last before the kit: four Spaces over accounts, `current` and
/// `recall` by position.
#[test]
fn the_last_file_before_the_kit_keeps_its_spaces_current_and_recall() {
    let (_dir, dirs) = dirs();
    let one = account(1);
    let thread = ThreadId::from_uuid(Uuid::from_u128(9));
    let space = |name: &str, scope: &str| {
        format!(
            r##"{{"name":"{name}","dots":[{{"hue":268.0,"chroma":0.5}}],"grain":0,"theme":"system","card_accent":"chosen","scope":{scope},"pins":[],"colors":{{"{one}":"#5B4FC4"}}}}"##
        )
    };
    let accounts = format!(r#"{{"kind":"accounts","v":["{one}"]}}"#);
    let text = format!(
        r#"{{"spaces":[{},{},{},{}],"current":1,"recall":{{"0":{{"place":"Sent","open":null,"account":null}},"1":{{"place":"Archive","open":"{thread}","account":"{one}"}}}}}}"#,
        space("A", &accounts),
        space("B", &accounts),
        space("C", &accounts),
        space("D", r#"{"kind":"all"}"#),
    );
    write(&dirs, &text);
    let read = load(&dirs);
    let names: Vec<&str> = read
        .list()
        .iter()
        .map(|space| space.name.as_str())
        .collect();
    assert_eq!(names, ["A", "B", "C", "D"]);
    assert_eq!(read.current().name, "B");
    assert_eq!(read.current().id, SpaceId(1), "ids are the old positions");
    assert_eq!(read.list()[0].payload.accounts, vec![one.clone()]);
    // Read only, with no store at hand, every account is none: the window's boot names them.
    assert_eq!(read.list()[3].payload.accounts, vec![]);
    // Each account's colour and the account tile left pressed are no longer kept, and are passed
    // over rather than failing the file.
    assert_eq!(
        read.recall().of(SpaceId(1)),
        Recall {
            place: "Archive".to_owned(),
            open: Some(thread),
        }
    );
    assert_eq!(read.recall().of(SpaceId(0)).place, "Sent");
}

/// What a Space holds that is mail's reads field by field: accounts that are not a list are
/// none, an entry that is not an account is passed over, and a field that is missing is its
/// default.
#[test]
fn mail_s_own_fields_read_leniently() {
    let (_dir, dirs) = dirs();
    write(
        &dirs,
        &format!(
            r#"{{"spaces":[{{"name":"A","accounts":"garbage"}},{{"name":"B","accounts":[7,"{}"]}},{{"name":"C","accounts":[]}}]}}"#,
            account(2)
        ),
    );
    let read = load(&dirs);
    let mails: Vec<Mail> = read
        .list()
        .iter()
        .map(|space| space.payload.clone())
        .collect();
    assert_eq!(
        mails,
        vec![Mail::default(), Mail::of(vec![account(2)]), Mail::default()]
    );
}

#[test]
fn a_space_belongs_to_the_default_profile_unless_it_says_otherwise() {
    let (_dir, dirs) = dirs();
    write(
        &dirs,
        r#"{"spaces":[{"name":"Old"},{"name":"Other","profile":"0123abcd"},{"name":"Bad","profile":"nope"}]}"#,
    );
    let profiles: Vec<ProfileId> = load(&dirs)
        .list()
        .iter()
        .map(|space| space.payload.profile.clone())
        .collect();
    assert_eq!(
        profiles,
        vec![ProfileId::default(), other_profile(), ProfileId::default()]
    );
}

fn other_profile() -> ProfileId {
    "0123abcd".parse().expect("a profile id")
}

#[test]
fn first_run_follows_the_accounts() {
    let none = first_run(&[]);
    assert_eq!(none.count(), 1, "zero accounts");
    assert_eq!(none.current().name, "Space 1");
    assert_eq!(none.current().look.dots, PRESETS[0].dots);
    assert_eq!(
        none.current().payload,
        Mail::default(),
        "a Space of no account"
    );

    let ids = [account(1), account(2), account(3)];
    let three = first_run(&ids);
    assert_eq!(three.count(), 3, "three accounts");
    assert_eq!(three.current().name, "Space 1");
    for (index, id) in ids.iter().enumerate() {
        let space = &three.list()[index];
        let name = format!("Space {}", index + 1);
        assert_eq!(space.name, name);
        assert_eq!(space.look.dots, PRESETS[index].dots, "{name}");
        assert_eq!(space.payload, Mail::of(vec![id.clone()]), "{name}");
    }
}

/// The window's boot: a first run is written; a stored Space with no theme takes the window-wide
/// one mailo kept before themes were the Spaces'.
#[test]
fn boot_writes_a_first_run_and_inherits_a_missing_theme() {
    let ids = [account(1), account(2)];
    let legacy = Legacy {
        theme: Theme::Dark,
        ..Legacy::default()
    };

    let (_dir, first) = dirs();
    let made = boot(Some(&first), &ids, &legacy);
    assert_eq!(made.count(), 2);
    assert!(
        made.list()
            .iter()
            .all(|space| space.look.theme == Theme::Dark)
    );
    assert_eq!(
        shown(&load(&first)),
        shown(&made),
        "the first run was not written"
    );

    let (_dir, stored) = dirs();
    write(
        &stored,
        r#"{"spaces":[{"name":"A","scope":{"kind":"all"}},{"name":"B","theme":"light"}]}"#,
    );
    let read = boot(Some(&stored), &ids, &legacy);
    let themes: Vec<Theme> = read.list().iter().map(|space| space.look.theme).collect();
    assert_eq!(
        themes,
        [Theme::Dark, Theme::Light],
        "only a missing theme inherits"
    );
}

#[test]
fn a_new_account_joins_the_space_it_is_put_in_and_only_one_of_its_profile() {
    let profile = ProfileId::default();
    let mut mail = Mail::of(vec![account(2)]);
    assert!(mail.widen(&profile, account(1)));
    assert!(!mail.widen(&profile, account(1)), "added twice");
    assert_eq!(mail.accounts, vec![account(2), account(1)]);
    assert_eq!(mail.scope(), Scope::Accounts(vec![account(2), account(1)]));

    let mut foreign = Mail {
        profile: other_profile(),
        ..Mail::of(vec![account(2)])
    };
    assert!(!foreign.widen(&profile, account(1)));
    assert_eq!(foreign.accounts, vec![account(2)]);
}

/// At boot, an account the store has and no Space holds (added from a terminal while no window
/// was open) stays in no Space, and an empty Space stays empty: accounts and Spaces are apart.
#[test]
fn boot_leaves_an_account_in_no_space_where_it_is() {
    let (_dir, stored) = dirs();
    write(
        &stored,
        &format!(
            r#"{{"spaces":[{{"name":"A","accounts":["{}"]}},{{"name":"B","accounts":[]}}],"current":1}}"#,
            account(1)
        ),
    );
    let read = boot(Some(&stored), &[account(1), account(2)], &Legacy::default());
    let accounts: Vec<Vec<AccountId>> = read
        .list()
        .iter()
        .map(|space| space.payload.accounts.clone())
        .collect();
    assert_eq!(accounts, vec![vec![account(1)], vec![]]);
}
