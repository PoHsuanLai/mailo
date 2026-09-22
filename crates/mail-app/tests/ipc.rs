//! The daemon's address, its wire, and how a client finds or starts it.
//!
//! `plan.md`'s next step: `mailo watch` already holds connections open for as long as the
//! process lives, and a daemon is that loop with a door in it. What is tested here is the door.
//!
//! The platform rules are tested *from whatever machine runs this*, because `endpoint_on` takes
//! the environment as arguments instead of reading it — the same shape `attach::downloads_from`
//! uses. A rule that reads its own environment can only be checked on the machine it was written
//! on, which for a cross-platform daemon is the one thing that must not be true.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

// `endpoint` and `reach` read the real environment, so nothing here calls them — a test that
// did would be talking to whatever daemon this user happens to have running, which is both a
// flaky test and a rude one.
#[allow(dead_code)]
#[path = "../src/ipc/mod.rs"]
mod ipc;

use ipc::{Endpoint, Environment, Host, endpoint_on};

fn os(text: &str) -> &OsStr {
    OsStr::new(text)
}

mod where_it_listens {
    use super::*;

    #[test]
    fn linux_uses_the_runtime_directory() {
        // tmpfs, 0700, and emptied when the session ends — which is the lifetime a socket wants,
        // and the reason a stale one cannot survive a reboot there.
        let found = endpoint_on(
            Host::Linux,
            &Environment {
                runtime_dir: Some(os("/run/user/1000")),
                home: Some(os("/home/ada")),
                ..Environment::default()
            },
        )
        .unwrap();
        assert_eq!(
            found,
            Endpoint::Socket(PathBuf::from("/run/user/1000/mailo/daemon.sock"))
        );
    }

    #[test]
    fn linux_without_a_runtime_directory_falls_back_under_home() {
        // A login shell over ssh often has no XDG_RUNTIME_DIR. Refusing to run there would make
        // the daemon unavailable in exactly the session most likely to want a CLI.
        let found = endpoint_on(
            Host::Linux,
            &Environment {
                runtime_dir: None,
                home: Some(os("/home/ada")),
                ..Environment::default()
            },
        )
        .unwrap();
        assert_eq!(
            found,
            Endpoint::Socket(PathBuf::from("/home/ada/.cache/mailo/daemon.sock"))
        );
    }

    #[test]
    fn macos_uses_its_own_per_user_temporary_directory() {
        // macOS has no XDG_RUNTIME_DIR at all. Its TMPDIR is per-user and private, which is the
        // property that matters; using /tmp instead would put the socket somewhere every other
        // account on the machine can see.
        let found = endpoint_on(
            Host::Mac,
            &Environment {
                tmpdir: Some(os("/var/folders/qw/8p3n1x/T")),
                home: Some(os("/Users/ada")),
                ..Environment::default()
            },
        )
        .unwrap();
        assert_eq!(
            found,
            Endpoint::Socket(PathBuf::from("/var/folders/qw/8p3n1x/T/mailo/daemon.sock"))
        );
    }

    #[test]
    fn macos_without_tmpdir_falls_back_into_the_library() {
        let found = endpoint_on(
            Host::Mac,
            &Environment {
                tmpdir: None,
                home: Some(os("/Users/ada")),
                ..Environment::default()
            },
        )
        .unwrap();
        assert_eq!(
            found,
            Endpoint::Socket(PathBuf::from("/Users/ada/Library/Caches/mailo/daemon.sock"))
        );
    }

    #[test]
    fn a_path_too_long_for_the_platform_is_refused_with_the_numbers() {
        // `sun_path` is 104 bytes on macOS, and the limit is not advisory — some libcs truncate
        // silently, which binds a socket somewhere nobody asked for. macOS is where this bites,
        // because its own TMPDIR is already long before anything is appended.
        // Chosen to land between the two limits: with `/mailo/daemon.sock` and the NUL this is
        // 107 bytes, which Linux allows and macOS does not. A length that failed on both would
        // not show that the limit is a property of the platform.
        let long = format!("/var/folders/{}", "x".repeat(75));
        let refused = endpoint_on(
            Host::Mac,
            &Environment {
                tmpdir: Some(OsStr::new(&long)),
                ..Environment::default()
            },
        )
        .expect_err("that cannot fit in sun_path");
        assert!(refused.contains("104"), "{refused}");

        // And the same path is fine on Linux, which allows four more bytes. The limit is a
        // property of the platform, not of the path.
        assert!(
            endpoint_on(
                Host::Linux,
                &Environment {
                    runtime_dir: Some(OsStr::new(&long)),
                    ..Environment::default()
                },
            )
            .is_ok()
        );
    }

    #[test]
    fn windows_names_a_pipe_after_the_user() {
        // A pipe name is machine-wide rather than per-session, so two people signed in to one
        // Windows box would otherwise be one daemon — reading each other's mail.
        let found = endpoint_on(
            Host::Windows,
            &Environment {
                user: Some("ada"),
                ..Environment::default()
            },
        )
        .unwrap();
        assert_eq!(found, Endpoint::Pipe(r"\\.\pipe\mailo-ada".to_owned()));
    }

    #[test]
    fn with_nothing_set_it_says_so_rather_than_guessing() {
        for host in [Host::Linux, Host::Mac] {
            endpoint_on(host, &Environment::default())
                .expect_err("there is nowhere to put a socket");
        }
    }
}

mod the_wire {
    use super::*;
    use ipc::wire::{self, Mismatch, Request, Response};

    #[test]
    fn a_request_survives_the_round_trip() {
        let text = wire::line(Request::SyncNow).unwrap();
        assert!(text.ends_with('\n'), "the framing is the newline");
        assert_eq!(wire::parse::<Request>(&text).unwrap(), Request::SyncNow);
    }

    #[test]
    fn a_message_never_contains_its_own_terminator() {
        // The framing is a newline, so a value carrying one would end the message early and the
        // rest would be read as the next one. JSON escapes them; this is the property the
        // framing depends on, asserted rather than assumed.
        let text = wire::line(Response::Refused("two\nlines".to_owned())).unwrap();
        assert_eq!(text.matches('\n').count(), 1, "{text:?}");
        assert_eq!(
            wire::parse::<Response>(&text).unwrap(),
            Response::Refused("two\nlines".to_owned())
        );
    }

    #[test]
    fn a_version_this_build_does_not_speak_is_refused_before_the_body_is_read() {
        // A daemon left running across an upgrade is the normal case — `watch` holds IDLE
        // connections for hours and nobody restarts it to install a binary — so the first thing
        // a new client meets is an old daemon. Reading the body under the old meaning of a
        // changed field is the failure this prevents.
        let stale = r#"{"version":999,"body":"ping"}"#;
        match wire::parse::<Request>(stale) {
            Err(Mismatch::Version { theirs, ours }) => {
                assert_eq!(theirs, 999);
                assert_eq!(ours, wire::VERSION);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_mismatch_says_which_way_round_it_is() {
        // Which side is older decides the remedy — restart the daemon, or upgrade the client —
        // so both numbers have to reach the person reading it.
        let said = Mismatch::Version { theirs: 2, ours: 1 }.to_string();
        assert!(said.contains('2') && said.contains('1'), "{said}");
        assert!(said.contains("restart"), "{said}");
    }

    #[test]
    fn rubbish_is_an_error_rather_than_a_panic() {
        assert!(matches!(
            wire::parse::<Request>("not json at all"),
            Err(Mismatch::Unreadable(_))
        ));
    }
}

/// Finding a daemon, and starting one when there is none.
mod finding_it {
    use super::*;
    use ipc::client;

    /// A listener that answers one `Ping` and then goes away.
    fn a_daemon_at(path: &Path) -> std::thread::JoinHandle<()> {
        let listener = ipc::bind(path).expect("the socket binds");
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader, Write};
            if let Some(Ok(mut stream)) = listener.incoming().next() {
                let mut line = String::new();
                let _ = BufReader::new(&stream).read_line(&mut line);
                let reply = ipc::wire::line(ipc::wire::Response::Pong {
                    pid: 4242,
                    version: "test".to_owned(),
                })
                .unwrap();
                let _ = stream.write_all(reply.as_bytes());
                let _ = stream.flush();
            }
        })
    }

    #[test]
    fn nothing_listening_is_not_an_error() {
        // The usual answer to "no daemon" is to start one, so it must be distinguishable from a
        // failure without parsing an error message.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.sock");
        assert!(client::connect_at(&path).unwrap().is_none());
    }

    #[test]
    fn a_socket_left_behind_by_a_dead_daemon_is_not_mistaken_for_a_live_one() {
        // The awkward case, and the only one that needs care: the process was killed, the file
        // is still there, and it looks exactly like a daemon until you try to talk to it. Only a
        // connection attempt can tell the difference.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.sock");
        let listener = ipc::bind(&path).unwrap();
        drop(listener); // the daemon dies, the file stays
        assert!(path.exists(), "the stale file is the whole point");

        assert!(
            client::connect_at(&path).unwrap().is_none(),
            "a file with nothing behind it was taken for a daemon"
        );
    }

    #[test]
    fn a_live_daemon_is_reused_rather_than_replaced() {
        // The property `mailo ping` depends on: asking twice must not start a second daemon
        // fetching the same mail twice.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.sock");
        let served = a_daemon_at(&path);

        let mut reached = client::connect_or_start(
            &path,
            || panic!("it started a daemon when one was already listening"),
            Duration::from_secs(1),
        )
        .expect("the running daemon answers");
        assert_eq!(
            reached.ask(ipc::wire::Request::Ping).unwrap(),
            ipc::wire::Response::Pong {
                pid: 4242,
                version: "test".to_owned()
            }
        );
        served.join().unwrap();
    }

    #[test]
    fn with_none_running_it_starts_one_and_waits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.sock");
        let starting = path.clone();

        let mut reached = client::connect_or_start(
            &path,
            move || {
                a_daemon_at(&starting);
                Ok(())
            },
            Duration::from_secs(5),
        )
        .expect("it started one and then reached it");
        assert!(matches!(
            reached.ask(ipc::wire::Request::Ping).unwrap(),
            ipc::wire::Response::Pong { .. }
        ));
    }

    #[test]
    fn a_daemon_that_never_answers_times_out_rather_than_hanging() {
        // A client blocked for ever on a daemon that failed to start is worse than one that
        // says so: the second can be retried by a person.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.sock");
        let refused = client::connect_or_start(&path, || Ok(()), Duration::from_millis(300))
            .expect_err("nothing ever listened");
        assert!(refused.contains("did not answer"), "{refused}");
    }

    #[test]
    fn binding_over_a_live_daemon_is_refused() {
        // Two daemons on one mailbox would fetch everything twice and race each other's writes.
        // The check is a *connection*, because the file existing proves nothing.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.sock");
        let _first = ipc::bind(&path).expect("the first one binds");
        let refused = ipc::bind(&path).expect_err("the second must not");
        assert!(refused.contains("already listening"), "{refused}");
    }

    #[test]
    fn binding_over_a_stale_socket_succeeds() {
        // The other half of the same rule: after a crash the file is still there, and refusing
        // to start because of it would mean a daemon that never comes back.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.sock");
        drop(ipc::bind(&path).unwrap());
        assert!(path.exists());
        ipc::bind(&path).expect("a stale socket is cleared, not obeyed");
    }
}
