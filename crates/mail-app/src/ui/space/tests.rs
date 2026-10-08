//! mailo's part of the Spaces: what a Space holds that is mail's, how the window boots them, and
//! that every `spaces.json` mailo ever wrote still reads as it did. How quire's kit reads a file
//! field by field is quire's to test; these are mailo's files through it.

use super::{Mail, Pinned, Recall, Scope, SpaceId, Spaces, boot, ensure_colors, first_run, load};
use crate::ui::appearance::{Legacy, WindowDirs};
use ds::prelude::{SpaceLook, Theme};
use ds::style::space::look::CardAccent;
use ds::style::space::palette::Dot;
use ds::style::space::presets::PRESETS;
use mail_domain::ThreadId;
use mail_domain::id::account_id_from_uuid;
use porter_core::AccountId;
use std::collections::BTreeMap;
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
/// card accent spelled `hint`, a `motion` nobody reads. It must still load to the same Spaces,
/// and a save must keep the flat shape and read back unchanged.
const BEFORE_QUIRE: &str = r#"{"spaces":[{"name":"Work","dots":[{"hue":268.0,"chroma":0.72},{"hue":318.0,"chroma":0.55}],"grain":35,"theme":"dark","motion":"calm","card_accent":"hint","scope":{"kind":"all"},"pins":[{"kind":"person","name":"Dana","email":"dana@example.com"}],"colors":{}},{"name":"Home","dots":[{"hue":152.0,"chroma":0.62}],"grain":55,"theme":"system","motion":"standard","card_accent":"postmark","scope":{"kind":"all"},"pins":[],"colors":{}}],"current":1,"recall":{}}
"#;

#[test]
fn a_spaces_file_from_before_quire_still_loads_and_round_trips() {
    let (_dir, dirs) = dirs();
    write(&dirs, BEFORE_QUIRE);
    let read = load(&dirs);
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
                card_accent: CardAccent::SpaceHue,
            },
            Mail {
                scope: Scope::All,
                pins: vec![Pinned::Person {
                    name: "Dana".to_owned(),
                    email: "dana@example.com".to_owned(),
                }],
                colors: BTreeMap::new(),
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
                card_accent: CardAccent::Chosen,
            },
            Mail::over(Scope::All),
        ),
    ];
    assert_eq!(shown(&read), want);
    assert_eq!(read.current().name, "Home", "`current` is a position");

    super::save(Some(&dirs), &read).unwrap_or_else(|e| panic!("{e}"));
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
    assert_eq!(work["card_accent"], "space_hue", "{work}");
    assert_eq!(work["scope"]["kind"], "all", "{work}");
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
    assert_eq!(
        read.list()[0].payload.scope,
        Scope::Accounts(vec![one.clone()])
    );
    assert_eq!(read.list()[3].payload.scope, Scope::All);
    assert_eq!(
        read.list()[0].payload.colors.get(&one).map(String::as_str),
        Some("#5B4FC4")
    );
    assert_eq!(
        read.recall().of(SpaceId(1)),
        Recall {
            place: "Archive".to_owned(),
            open: Some(thread),
            account: Some(one),
        }
    );
    assert_eq!(read.recall().of(SpaceId(0)).place, "Sent");
}

/// What a Space holds that is mail's reads field by field: a scope this build does not know is
/// every account, and a field that is missing is its default.
#[test]
fn mail_s_own_fields_read_leniently() {
    let (_dir, dirs) = dirs();
    write(
        &dirs,
        r#"{"spaces":[{"name":"A","scope":{"kind":"team","v":"x"}},{"name":"B","scope":"garbage"},{"name":"C"}]}"#,
    );
    let read = load(&dirs);
    let mails: Vec<Mail> = read
        .list()
        .iter()
        .map(|space| space.payload.clone())
        .collect();
    assert_eq!(mails, vec![Mail::default(); 3]);
}

#[test]
fn first_run_follows_the_accounts() {
    let none = first_run(&[]);
    assert_eq!(none.count(), 1, "zero accounts");
    assert_eq!(none.current().name, "Space 1");
    assert_eq!(none.current().look.dots, PRESETS[0].dots);
    assert_eq!(none.current().payload, Mail::over(Scope::All));

    let ids = [account(1), account(2), account(3)];
    let three = first_run(&ids);
    assert_eq!(three.count(), 3, "three accounts");
    assert_eq!(three.current().name, "Space 1");
    for (index, id) in ids.iter().enumerate() {
        let space = &three.list()[index];
        let name = format!("Space {}", index + 1);
        assert_eq!(space.name, name);
        assert_eq!(space.look.dots, PRESETS[index].dots, "{name}");
        assert_eq!(
            space.payload,
            Mail::over(Scope::Accounts(vec![id.clone()])),
            "{name}"
        );
    }
}

/// The window's boot: a first run is written; a stored Space with no theme takes the window-wide
/// one mailo kept before themes were the Spaces'; the current Space's accounts get a colour each.
#[test]
fn boot_writes_a_first_run_inherits_a_missing_theme_and_colours_the_accounts() {
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
    let colours = &read.current().payload.colors;
    assert!(ids.iter().all(|id| colours.contains_key(id)), "{colours:?}");
    assert_eq!(
        load(&stored).current().payload.colors,
        *colours,
        "the colours were not written"
    );
}

#[test]
fn colours_fill_only_the_accounts_without_one() {
    let mut mail = Mail::default();
    mail.colors.insert(account(1), "#000000".to_owned());
    assert!(ensure_colors(&mut mail, &[account(1), account(2)]));
    assert_eq!(mail.colors[&account(1)], "#000000");
    assert!(mail.colors.contains_key(&account(2)));
    assert!(
        !ensure_colors(&mut mail, &[account(1), account(2)]),
        "twice"
    );
}

#[test]
fn a_new_account_joins_a_scoped_space_and_not_an_open_one() {
    let mut open = Mail::over(Scope::All);
    assert!(!open.widen(account(1)));
    assert_eq!(open.scope, Scope::All);

    let mut scoped = Mail::over(Scope::Accounts(vec![account(2)]));
    assert!(scoped.widen(account(1)));
    assert!(!scoped.widen(account(1)), "added twice");
    assert_eq!(scoped.scope, Scope::Accounts(vec![account(2), account(1)]));
}
