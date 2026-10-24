//! The registry reads no file into one default Profile and writes nothing, keeps what it is told
//! across a save, and never lets an id that is not one onto disk.

use super::*;
use std::collections::BTreeSet;

fn listing(dir: &Path) -> Vec<PathBuf> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    names.sort();
    names
}

#[test]
fn a_missing_registry_is_one_default_profile_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let before = listing(dir.path());

    let registry = Registry::load(dir.path()).unwrap();

    let [only] = registry.profiles() else {
        panic!("{:?}", registry.profiles());
    };
    assert!(only.id().is_default());
    assert_eq!(only.home(), Home::Legacy);
    assert_eq!(only.name(), None);
    assert!(registry.file.removing.is_empty());
    assert_eq!(listing(dir.path()), before);
}

#[test]
fn added_and_renamed_profiles_survive_a_save_with_fresh_valid_ids() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path()).unwrap();
    let work = registry.add("Work").unwrap();
    let ids: BTreeSet<_> = (0..40)
        .map(|n| registry.add(&format!("Side {n}")).unwrap().id().clone())
        .chain([work.id().clone()])
        .collect();
    assert_eq!(ids.len(), 41, "every id is new");
    for id in &ids {
        assert!(!id.is_default());
        assert_eq!(id.as_str().parse::<ProfileId>().as_ref(), Ok(id));
    }
    registry.rename(work.id(), "Office").unwrap();
    registry.rename(&ProfileId::default(), "Home").unwrap();
    registry.save().unwrap();

    let again = Registry::load(dir.path()).unwrap();
    assert_eq!(again.profiles(), registry.profiles());
    assert_eq!(again.get(work.id()).unwrap().name(), Some("Office"));
    assert_eq!(again.profiles()[0].name(), Some("Home"));
}

#[test]
fn a_profile_is_found_by_its_id_or_its_name_in_any_case() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path()).unwrap();
    let work = registry.add("Work").unwrap();

    assert_eq!(registry.resolve(work.id().as_str()).unwrap(), &work);
    assert_eq!(
        registry
            .resolve(&work.id().as_str().to_uppercase())
            .unwrap(),
        &work
    );
    assert_eq!(registry.resolve("wORK").unwrap(), &work);
    assert!(registry.resolve("Default").unwrap().id().is_default());
    assert!(matches!(
        registry.resolve("Play"),
        Err(CoreError::NoProfile(_))
    ));

    // A name a lookup could not tell from an id, or from another Profile's, is refused.
    for name in ["", "  ", "default", "DEADBEEF"] {
        assert!(
            matches!(registry.add(name), Err(CoreError::ProfileNameUnusable(_))),
            "{name:?}"
        );
    }
    assert!(matches!(
        registry.add(" work "),
        Err(CoreError::ProfileNameTaken(_))
    ));
    registry.rename(work.id(), "WORK").unwrap();
}

#[test]
fn an_id_that_is_not_one_is_refused_when_parsed_and_when_read() {
    for text in [
        "",
        "deadbee",
        "deadbeef0",
        "DEADBEEF",
        "ghijklmn",
        "../../x",
        "Default",
    ] {
        assert_eq!(
            text.parse::<ProfileId>(),
            Err(NotAProfileId(text.to_owned()))
        );
    }
    assert!("deadbeef".parse::<ProfileId>().is_ok());

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("profiles.json"),
        r#"{"version":1,"profiles":[{"id":"../elsewhere","name":"Work"}]}"#,
    )
    .unwrap();
    assert!(Registry::load(dir.path()).is_err());
}

#[test]
fn a_removal_is_remembered_until_it_is_finished_and_the_default_cannot_be_removed() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = Registry::load(dir.path()).unwrap();
    let work = registry.add("Work").unwrap();
    registry.save().unwrap();

    assert!(matches!(
        registry.begin_remove(&ProfileId::default()),
        Err(CoreError::DefaultProfileStays)
    ));
    registry.begin_remove(work.id()).unwrap();
    let mut again = Registry::load(dir.path()).unwrap();
    assert_eq!(again.file.removing, [work.id().clone()]);
    assert!(
        again.get(work.id()).is_some(),
        "listed until the removal finishes"
    );

    again.finish_remove(work.id()).unwrap();
    let after = Registry::load(dir.path()).unwrap();
    assert!(after.file.removing.is_empty());
    assert!(after.get(work.id()).is_none());
    assert_eq!(after.profiles().len(), 1);
}

#[test]
fn the_default_profile_is_the_install_in_place_and_another_has_a_folder() {
    let roots = Roots {
        data: PathBuf::from("/data/mailo"),
        config: PathBuf::from("/config/mailo"),
    };
    let default = Profile {
        id: ProfileId::default(),
        name: None,
    };
    assert_eq!(default.db(&roots), Path::new("/data/mailo/mail.db"));
    assert_eq!(default.blobs(&roots), Path::new("/data/mailo/blobs"));
    assert_eq!(default.signing_service(), "mailo");

    let work = Profile {
        id: "0a1b2c3d".parse().unwrap(),
        name: Some("Work".to_owned()),
    };
    assert_eq!(work.home(), Home::Folder);
    assert_eq!(
        work.db(&roots),
        Path::new("/data/mailo/profiles/0a1b2c3d/mail.db")
    );
    assert_eq!(
        work.blobs(&roots),
        Path::new("/data/mailo/profiles/0a1b2c3d/blobs")
    );
    assert_eq!(work.signing_service(), "mailo/0a1b2c3d");
}
