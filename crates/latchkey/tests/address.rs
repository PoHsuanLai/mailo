//! Where each platform puts its agent.
//!
//! All of it runs on every machine, because [`address_on`] takes the environment as arguments
//! instead of reading it. That is the property the crate is built around: the macOS rule is
//! checked by whoever runs `cargo test`, not by whoever happens to own a Mac.

use latchkey::{Endpoint, Environment, Error, Host, address_on};
use std::ffi::OsStr;
use std::path::PathBuf;

fn os(text: &str) -> &OsStr {
    OsStr::new(text)
}

#[test]
fn linux_uses_the_runtime_directory() {
    // tmpfs, 0700, and emptied when the session ends — the lifetime a socket wants, and the
    // reason a stale one there cannot survive a reboot.
    let found = address_on(
        "mailo",
        Host::Linux,
        &Environment {
            runtime_dir: Some(os("/run/user/1000")),
            home: Some(os("/home/ada")),
            ..Environment::default()
        },
    )
    .unwrap();
    assert_eq!(
        found.endpoint,
        Endpoint::Socket(PathBuf::from("/run/user/1000/mailo/agent.sock"))
    );
    assert_eq!(found.lock, PathBuf::from("/run/user/1000/mailo/agent.lock"));
}

#[test]
fn linux_without_a_runtime_directory_falls_back_under_home() {
    // A login shell over ssh often has no XDG_RUNTIME_DIR. Refusing to run there would make the
    // agent unavailable in exactly the session most likely to want a command-line client.
    let found = address_on(
        "mailo",
        Host::Linux,
        &Environment {
            home: Some(os("/home/ada")),
            ..Environment::default()
        },
    )
    .unwrap();
    assert_eq!(
        found.endpoint,
        Endpoint::Socket(PathBuf::from("/home/ada/.cache/mailo/agent.sock"))
    );
}

#[test]
fn macos_uses_its_own_per_user_temporary_directory() {
    // macOS has no XDG_RUNTIME_DIR at all. Its TMPDIR is per-user and private, which is the
    // property that matters; /tmp would put the socket somewhere every other account can see.
    let found = address_on(
        "mailo",
        Host::Mac,
        &Environment {
            tmpdir: Some(os("/var/folders/qw/8p3n1x/T")),
            home: Some(os("/Users/ada")),
            ..Environment::default()
        },
    )
    .unwrap();
    assert_eq!(
        found.endpoint,
        Endpoint::Socket(PathBuf::from("/var/folders/qw/8p3n1x/T/mailo/agent.sock"))
    );
}

#[test]
fn macos_without_a_temporary_directory_falls_back_to_its_caches() {
    let found = address_on(
        "mailo",
        Host::Mac,
        &Environment {
            home: Some(os("/Users/ada")),
            ..Environment::default()
        },
    )
    .unwrap();
    assert_eq!(
        found.endpoint,
        Endpoint::Socket(PathBuf::from("/Users/ada/Library/Caches/mailo/agent.sock"))
    );
}

#[test]
fn windows_names_the_pipe_after_the_user() {
    // One pipe namespace for the whole machine, so the name has to carry the user: two people
    // signed in to one Windows box are two agents, and without this they would be one — sharing
    // whatever the agent holds.
    let found = address_on(
        "mailo",
        Host::Windows,
        &Environment {
            local_app_data: Some(os(r"C:\Users\ada\AppData\Local")),
            user: Some("ada"),
            ..Environment::default()
        },
    )
    .unwrap();
    assert_eq!(found.endpoint, Endpoint::Pipe("mailo-ada".to_owned()));
    assert_eq!(
        found.lock,
        PathBuf::from(r"C:\Users\ada\AppData\Local")
            .join("mailo")
            .join("agent.lock")
    );
}

#[test]
fn two_users_on_one_windows_machine_are_two_agents() {
    let for_user = |who: &str| {
        address_on(
            "mailo",
            Host::Windows,
            &Environment {
                local_app_data: Some(os(r"C:\x")),
                user: Some(who),
                ..Environment::default()
            },
        )
        .unwrap()
        .endpoint
    };
    assert_ne!(for_user("ada"), for_user("grace"));
}

#[test]
fn a_path_too_long_for_sun_path_is_refused_with_the_numbers() {
    // `sun_path` is 108 bytes on Linux and 104 on macOS, and the limit is not advisory: a longer
    // path is silently truncated by some libcs and refused by others, and a truncated one binds
    // somewhere nobody asked for. Refusing loudly is the only safe answer.
    //
    // This directory is 88 characters, and "/mailo/agent.sock" adds 17, so the whole is 106
    // bytes including the NUL — which fits Linux and does not fit macOS. One environment, two
    // answers, which is exactly the case a single `cfg!` would have hidden. Asserted rather than
    // eyeballed, because the first version of this test used a length that fit both and passed
    // for the wrong reason.
    let long = "/".to_owned() + &"d".repeat(87);
    assert_eq!(long.len() + "/mailo/agent.sock".len() + 1, 106);
    let env = Environment {
        runtime_dir: Some(os(&long)),
        tmpdir: Some(os(&long)),
        ..Environment::default()
    };

    address_on("mailo", Host::Linux, &env).expect("108 bytes is enough");
    match address_on("mailo", Host::Mac, &env) {
        Err(Error::TooLong { length, limit, .. }) => {
            assert_eq!(limit, 104);
            assert!(length > 104, "{length}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_refusal_says_enough_to_act_on() {
    let long = "/".to_owned() + &"d".repeat(120);
    let said = address_on(
        "mailo",
        Host::Mac,
        &Environment {
            tmpdir: Some(os(&long)),
            ..Environment::default()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(said.contains("104"), "{said}");
    assert!(said.contains("mailo"), "{said}");
}

#[test]
fn a_name_that_could_escape_its_directory_is_refused() {
    // The name becomes a path component on two platforms and part of a machine-wide namespace on
    // the third. Escaping it would be a guess about three sets of rules; refusing is not.
    for bad in ["", "../etc", "a/b", "a b", r"a\b", "a.b", "naïve"] {
        address_on(
            bad,
            Host::Linux,
            &Environment {
                runtime_dir: Some(os("/run/user/1000")),
                ..Environment::default()
            },
        )
        .expect_err(bad);
    }
    for good in ["mailo", "my-agent", "agent2"] {
        address_on(
            good,
            Host::Linux,
            &Environment {
                runtime_dir: Some(os("/run/user/1000")),
                ..Environment::default()
            },
        )
        .expect(good);
    }
}

#[test]
fn with_nothing_set_it_says_so_rather_than_guessing() {
    for host in [Host::Linux, Host::Mac, Host::Windows] {
        let said = address_on("mailo", host, &Environment::default())
            .expect_err("there is nowhere to put a socket")
            .to_string();
        assert!(said.contains("nowhere"), "{host:?}: {said}");
    }
}
