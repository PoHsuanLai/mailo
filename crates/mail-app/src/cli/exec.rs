//! What each command does once `main` has chosen it: the steps around the store.
//!
//! [`run`](super::run) is synchronous and tested without a network, a runtime or a person at a
//! terminal. Everything that needs one of those lives here instead: [`prepare`] before the store
//! is opened (discovery, which asks a person; the body read from stdin), and [`execute`] after
//! (sync and watch, the daemon, the commands that wait on the network, and the exit code).

use super::Command;
use mail_core::SqliteStore;
use std::sync::Arc;

/// What [`prepare`] leaves to be done.
#[derive(Debug)]
pub enum Prepared {
    /// The command was answered before any store was opened.
    Finished,
    /// Go on with this command, or with the window when there is none.
    Run(Option<Command>),
}

/// The steps that come before the store is opened.
pub fn prepare(command: Option<Command>) -> Prepared {
    // The schema needs nothing but somewhere to write it: no store, no config directory.
    if let Some(Command::WriteSchema { dir }) = &command {
        match crate::settings::write_schema(dir) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return Prepared::Finished;
    }

    // Discovery needs the network and, unless `--yes` was given, a person at a terminal to say
    // yes to what it found; both are here rather than in `cli::run`, which is synchronous and
    // tested without either. Nothing is stored and nothing is sent to a found server before
    // the yes.
    if let Some(Command::AccountDiscover { address }) = &command {
        match super::discover::show(address, super::discover::lookup) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return Prepared::Finished;
    }
    let command = match command {
        Some(command) => match super::discover::before_add(
            command,
            super::discover::lookup,
            super::discover::Terminal::of_stdin(),
            |text| {
                use std::io::Write as _;
                print!("{text}");
                let _ = std::io::stdout().flush();
            },
            |question| {
                use std::io::Write as _;
                print!("{question}");
                let _ = std::io::stdout().flush();
                let mut line = String::new();
                match std::io::stdin().read_line(&mut line) {
                    Ok(0) | Err(_) => None,
                    Ok(_) => Some(line),
                }
            },
        ) {
            Ok(command) => Some(command),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        },
        None => None,
    };
    // `--jmap` with no URL: the domain's well-known session, found and confirmed the same way.
    let command = match command {
        Some(command) => match super::discover::before_add_jmap(
            command,
            |domain: &str| {
                crate::edge::block_on(mail_core::discover::find_jmap(domain)).map_err(String::from)
            },
            super::discover::Terminal::of_stdin(),
            |text| {
                use std::io::Write as _;
                print!("{text}");
                let _ = std::io::stdout().flush();
            },
            |question| {
                use std::io::Write as _;
                print!("{question}");
                let _ = std::io::stdout().flush();
                let mut line = String::new();
                match std::io::stdin().read_line(&mut line) {
                    Ok(0) | Err(_) => None,
                    Ok(_) => Some(line),
                }
            },
        ) {
            Ok(command) => Some(command),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        },
        None => None,
    };

    // `reply` takes its body from stdin, which is I/O and so does not belong in the parser.
    // Read here, once, before anything opens the database.
    let command = match command {
        Some(Command::Reply {
            message,
            scope,
            body: _,
        }) => {
            let mut body = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body) {
                eprintln!("cannot read the message body: {e}");
                std::process::exit(1);
            }
            Some(Command::Reply {
                message,
                scope,
                body,
            })
        }
        Some(Command::Compose {
            from,
            to,
            cc,
            bcc,
            subject,
            body: _,
            receipt,
            openpgp,
            smime,
        }) => {
            let mut body = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body) {
                eprintln!("cannot read the message body: {e}");
                std::process::exit(1);
            }
            Some(Command::Compose {
                from,
                to,
                cc,
                bcc,
                subject,
                body,
                receipt,
                openpgp,
                smime,
            })
        }
        Some(Command::Forward {
            message,
            to,
            body: _,
            carry,
        }) => {
            let mut body = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body) {
                eprintln!("cannot read the covering note: {e}");
                std::process::exit(1);
            }
            Some(Command::Forward {
                message,
                to,
                body,
                carry,
            })
        }
        // `signature` takes its text from stdin too, unless it is being cleared.
        Some(Command::Signature {
            address,
            clear: false,
            text: _,
        }) => {
            let mut text = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut text) {
                eprintln!("cannot read the signature: {e}");
                std::process::exit(1);
            }
            Some(Command::Signature {
                address,
                clear: false,
                text,
            })
        }
        other => other,
    };
    // `print` with no `--out` goes to stdout when that is a pipe or a file, and to a file
    // when it is a terminal. Only here can that be asked.
    let command = match command {
        Some(Command::Print {
            target,
            out: super::PrintTo::Unsaid,
            pages,
        }) => {
            use std::io::IsTerminal as _;
            let out = if std::io::stdout().is_terminal() {
                super::PrintTo::Into(std::path::PathBuf::from("."))
            } else {
                super::PrintTo::Stdout
            };
            Some(Command::Print { target, out, pages })
        }
        other => other,
    };
    Prepared::Run(command)
}

/// Do `command`, ending the process with a failing status when it fails.
pub fn execute(store: Arc<SqliteStore>, command: Command) {
    // Sync needs an async runtime and the store by Arc, so it is dispatched here rather than
    // inside super::run, which is deliberately synchronous and testable.
    if matches!(command, Command::Sync) {
        let mail = crate::edge::mail(&store);
        match crate::edge::block_on(mail.sync().run(Default::default())) {
            Ok(ends) => print!("{}", super::sync::run_text(&store, &ends)),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Command::SyncFolder { account, path } = &command {
        match super::sync_folder(store, account, path) {
            Ok(text) => print!("{text}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    // Saving an attachment may have to download it first — a large IMAP message's attachments
    // stay on the server until asked for — and that needs the store by `Arc`, like sync.
    if let Command::Save {
        message,
        index,
        dir,
    } = &command
    {
        let mail = crate::edge::mail(&store);
        let download =
            |section: &str| crate::edge::block_on(mail.sync().fetch_part(*message, section));
        match mail_core::attach::fetch_and_save(&store, *message, *index, dir, download) {
            Ok(path) => println!("{}", super::attach::saved(&path)),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    // The daemon and the clients that reach it. Dispatched here with `sync` and `watch` because
    // they need the store by `Arc` and an exit code, neither of which `super::run` has.
    match &command {
        Command::Daemon { stop: false } => {
            match mail_core::ipc::daemon::serve(
                store,
                std::sync::Arc::new(|store| {
                    let mail = crate::edge::mail(&store);
                    match crate::edge::block_on(mail.sync().run(Default::default())) {
                        Ok(ends) => {
                            print!("{}", super::sync::run_text(&store, &ends));
                            ends.iter()
                                .filter(|end| end.may_have_stored())
                                .map(|end| end.account())
                                .collect()
                        }
                        Err(why) => {
                            eprintln!("{why}");
                            Vec::new()
                        }
                    }
                }),
                |listening| match listening {
                    mail_core::ipc::daemon::Listening::Socket(path) => {
                        println!("listening on {}", path.display());
                    }
                    mail_core::ipc::daemon::Listening::Endpoint(endpoint) => {
                        println!("listening on {endpoint}");
                    }
                },
            ) {
                Ok(ended) => {
                    if ended == mail_core::ipc::daemon::Ended::Stopped {
                        println!("stopped");
                    }
                    return;
                }
                Err(message) => {
                    eprintln!("{message}");
                    std::process::exit(1);
                }
            }
        }
        Command::Daemon { stop: true } => {
            // Never starts one in order to stop it, which is why this is `connect` and not
            // `reach`: "there was nothing to stop" is a success, not a reason to spawn a daemon
            // and immediately ask it to leave.
            match mail_core::ipc::client::connect() {
                Ok(None) => println!("no daemon is running"),
                Ok(Some(mut daemon)) => match daemon.ask(mail_core::ipc::wire::Request::Shutdown) {
                    Ok(mail_core::ipc::wire::Response::Stopping) => println!("stopped"),
                    Ok(other) => println!("{other:?}"),
                    Err(why) => {
                        eprintln!("{why}");
                        std::process::exit(1);
                    }
                },
                Err(why) => {
                    eprintln!("{why}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Command::Ping => {
            match mail_core::ipc::client::reach()
                .and_then(|mut d| d.ask(mail_core::ipc::wire::Request::Ping))
            {
                Ok(mail_core::ipc::wire::Response::Pong { pid, version }) => {
                    println!("daemon {version} answering, pid {pid}");
                }
                Ok(other) => println!("{other:?}"),
                Err(why) => {
                    eprintln!("{why}");
                    std::process::exit(1);
                }
            }
            return;
        }
        _ => {}
    }

    // A Web Key Directory lookup needs the network: dispatched here with the other commands that
    // do, so `super::run` stays something a test can call without one.
    if let Command::Pgp(super::pgp::PgpCommand::Lookup { address }) = &command {
        match crate::edge::block_on(mail_core::pgp::lookup_address(
            &store,
            address,
            chrono::Utc::now(),
        )) {
            Ok(found) => print!("{}", super::pgp::lookup(found.as_ref(), address)),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    // An encrypted message to someone with no key yet: their domain is asked, once, as the
    // draft is made — the other moment the brief names besides an explicit lookup.
    if let Command::Compose {
        to, cc, openpgp, ..
    } = &command
        && openpgp.encrypts()
    {
        let addresses: Vec<String> = to.iter().chain(cc).map(|a| a.email.clone()).collect();
        let found = crate::edge::block_on(mail_core::pgp::discover(
            &store,
            &addresses,
            chrono::Utc::now(),
        ));
        eprint!("{}", super::pgp::discovered(&found));
    }

    // Import and export print progress as they go, to stderr, and an upload needs the network:
    // dispatched here for the same reasons as `sync`.
    if let Command::Import { path, into } = &command {
        match super::import::run(&store, path, into) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Command::Export { query, target } = &command {
        match super::export::run(&store, query, target) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }

    if let Command::Notify { set } = &command {
        match super::settings::notify(crate::settings::person_root().as_ref(), *set) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Command::Offline { address, set } = &command {
        let accounts = mail_core::sync::addresses(&store);
        match super::offline::command(
            mail_core::config::config_dir().as_deref(),
            store.as_ref(),
            &accounts,
            address.as_deref(),
            *set,
        ) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Command::Intents = &command {
        // What the router starts by D-Bus activation: no window, the same store. A second one
        // finds the name taken and ends quietly (exit 0), so a start that races another is not
        // a failure.
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let provider = crate::intents::Provider::new(
                store.clone(),
                std::sync::Arc::new(mail_core::KeyringSigningStore::default()),
                crate::intents::Opener::window(),
            );
            match crate::intents::serve(provider) {
                Ok(()) | Err(crate::intents::ServeError::Taken) => return,
                Err(why) => {
                    eprintln!("{why}");
                    std::process::exit(1);
                }
            }
        }
        #[cfg(any(target_os = "macos", windows))]
        {
            eprintln!("mailo intents answers on the Linux session bus, which this system has not");
            std::process::exit(1);
        }
    }
    // `watch` is `sync` that does not stop. It prints as it goes rather than at the end, because
    // "at the end" is when the user presses Ctrl-C.
    if let Command::Watch { notify } = &command {
        let notifications = match notify {
            super::WatchNotify::Never => mail_core::notify::Setting::Off,
            super::WatchNotify::AsSet => mail_core::notify::Setting::from(
                crate::settings::person_root()
                    .map(|root| crate::settings::load(&root))
                    .unwrap_or_default()
                    .notifications
                    .new_mail,
            ),
        };
        // One watch per user: a second would announce every message twice. A unit that starts
        // while one runs by hand ends quietly (exit 0), so systemd does not restart it in a loop.
        let watching = match mail_core::ipc::watching::claim() {
            Ok(held) => held,
            Err(mail_core::ipc::watching::Refused::AlreadyWatching) => {
                println!("a mailo watch is already running; leaving it to it");
                return;
            }
            Err(mail_core::ipc::watching::Refused::Failed(why)) => {
                eprintln!("cannot tell whether a mailo watch is running: {why}");
                std::process::exit(1);
            }
        };
        if let Some(why) = watching.doorless() {
            eprintln!("an open window will look for new mail rather than be told of it: {why}");
        }
        // The unread count on the launcher, kept up whether or not a window is open.
        if let Some(Err(e)) = crate::session::keep_the_launchers_badge(store.clone()) {
            eprintln!("the launcher's unread count is off: {e}");
        }
        println!("watching. Ctrl-C to stop.");
        // Said as it happens, and flushed: a watch is read by someone waiting on it. A window
        // listening at the watch's door is told too, so it reads what was stored now rather than
        // at its next look.
        let say = |watched: mail_core::sync::report::Watched| {
            use std::io::Write as _;
            if let mail_core::sync::report::Watched::Pass(end) = &watched
                && end.may_have_stored()
            {
                watching.changed(end.account());
            }
            print!("{}", super::sync::watched_text(&watched));
            let _ = std::io::stdout().flush();
        };
        let mail = crate::edge::mail(&store);
        // The desktop's notification service when notifications are on; with none to reach it
        // says nothing, and the watch goes on fetching.
        let desktop;
        let notifier: Option<&dyn mail_core::notify::Notifier> = match notifications {
            mail_core::notify::Setting::On => {
                desktop = mail_core::notify::desktop::Desktop::connect();
                Some(&desktop)
            }
            mail_core::notify::Setting::Off => None,
        };
        match crate::edge::block_on(mail.sync().watch(notifier, &say)) {
            // Only reached when every account has stopped for a reason worth stopping for — a
            // credential the server refused, which no amount of retrying fixes.
            Ok(ends) => {
                print!("{}", super::sync::run_text(&store, &ends));
                eprintln!("stopped watching; nothing left to watch");
                std::process::exit(1);
            }
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
    }

    match super::run(&store, &command, chrono::Utc::now()) {
        Ok(output) => print!("{output}"),
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}
