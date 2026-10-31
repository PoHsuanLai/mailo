//! What each command does once `main` has chosen it: the steps around the store.
//!
//! [`run`](super::run) is synchronous and tested without a network, a runtime or a person at a
//! terminal. Everything that needs one of those lives here instead: [`prepare`] before the store
//! is opened (discovery, which asks a person; the body read from stdin), and [`execute`] after
//! (sync and watch, the daemon, the commands that wait on the network). Both return what went
//! wrong rather than ending the process: `main` prints it, once, and exits with a failing status.

use super::Command;
use mail_core::SqliteStore;
use std::sync::Arc;
use std::time::Duration;

/// How long a watch waits before asking for the lock again, and before starting again after it
/// ended on an error: `mailo watch --wait`'s, and the app's own watch (`mail-app`'s `background`)
/// keeps the same pace.
pub const AGAIN: Duration = Duration::from_secs(60);

/// What [`prepare`] leaves to be done.
#[derive(Debug)]
pub enum Prepared {
    /// The command was answered before any store was opened.
    Finished,
    /// Go on with this command, or with the window when there is none.
    Run(Option<Command>),
}

/// The steps that come before the store is opened.
pub fn prepare(command: Option<Command>) -> Result<Prepared, String> {
    // Help and the version need nothing at all.
    match &command {
        Some(Command::Help) => {
            print!("{}", super::usage());
            return Ok(Prepared::Finished);
        }
        Some(Command::Version) => {
            print!("{}", super::version());
            return Ok(Prepared::Finished);
        }
        _ => {}
    }
    // The schema needs nothing but somewhere to write it: no store, no config directory.
    if let Some(Command::WriteSchema { dir }) = &command {
        print!("{}", mail_ui_model::settings::write_schema(dir)?);
        return Ok(Prepared::Finished);
    }

    // Discovery needs the network and, unless `--yes` was given, a person at a terminal to say
    // yes to what it found; both are here rather than in `cli::run`, which is synchronous and
    // tested without either. Nothing is stored and nothing is sent to a found server before
    // the yes.
    if let Some(Command::AccountDiscover { address }) = &command {
        print!(
            "{}",
            super::discover::show(address, super::discover::lookup)?
        );
        return Ok(Prepared::Finished);
    }
    let command = command
        .map(|command| {
            super::discover::before_add(
                command,
                super::discover::lookup,
                super::discover::Terminal::of_stdin(),
                say,
                ask,
            )
        })
        .transpose()?;
    // `--jmap` with no URL: the domain's well-known session, found and confirmed the same way.
    let command = command
        .map(|command| {
            super::discover::before_add_jmap(
                command,
                |domain: &str| {
                    mail_edge::block_on(mail_core::discover::find_jmap(domain))
                        .map_err(String::from)
                },
                super::discover::Terminal::of_stdin(),
                say,
                ask,
            )
        })
        .transpose()?;

    // `reply` takes its body from stdin, which is I/O and so does not belong in the parser.
    // Read here, once, before anything opens the database.
    let command = match command {
        Some(Command::Reply {
            message,
            scope,
            body: _,
        }) => Some(Command::Reply {
            message,
            scope,
            body: stdin("the message body")?,
        }),
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
        }) => Some(Command::Compose {
            from,
            to,
            cc,
            bcc,
            subject,
            body: stdin("the message body")?,
            receipt,
            openpgp,
            smime,
        }),
        Some(Command::Forward {
            message,
            to,
            body: _,
            carry,
        }) => Some(Command::Forward {
            message,
            to,
            body: stdin("the covering note")?,
            carry,
        }),
        // `signature` takes its text from stdin too, unless it is being cleared.
        Some(Command::Signature {
            address,
            clear: false,
            text: _,
        }) => Some(Command::Signature {
            address,
            clear: false,
            text: stdin("the signature")?,
        }),
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
    Ok(Prepared::Run(command))
}

/// All of stdin, which is `what` the command was given there.
fn stdin(what: &str) -> Result<String, String> {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
        .map_err(|e| format!("can't read {what}: {e}"))?;
    Ok(text)
}

/// Print what discovery found, before it asks about it.
fn say(text: &str) {
    use std::io::Write as _;
    print!("{text}");
    // A terminal that cannot be flushed shows the words late; the question still waits.
    let _ = std::io::stdout().flush();
}

/// Ask `question` and read the answer, or `None` when stdin has ended or cannot be read.
fn ask(question: &str) -> Option<String> {
    say(question);
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line),
    }
}

/// What the daemon answered, when it was not the answer asked for.
fn answered(response: &mail_core::ipc::wire::Response) -> String {
    use mail_core::ipc::wire::Response;
    match response {
        Response::Pong { pid, version } => format!("daemon {version} answering, pid {pid}"),
        Response::Started => "the daemon has started a sync".to_owned(),
        Response::Stopping => "the daemon is stopping".to_owned(),
        Response::Refused(why) => format!("the daemon refused: {why}"),
        Response::WrongVersion { daemon, client } => format!(
            "the daemon running speaks version {daemon} of its protocol and this mailo version \
             {client}: one of them is from an older mailo"
        ),
    }
}

/// Whether `command` reads `settings.toml` or `offline.json`: a sync keeps what `offline.json`
/// says, `watch` and `notify` read the notifications, and adding or removing an account writes
/// `offline.json`.
fn reads_settings(command: &Command) -> bool {
    matches!(
        command,
        Command::Sync
            | Command::SyncFolder { .. }
            | Command::Watch { .. }
            | Command::Notify { .. }
            | Command::AccountAdd { .. }
            | Command::AccountRemove { .. }
    )
}

/// Do `command`, or say what went wrong.
///
/// A command that reads a settings file that is there and cannot be used says which and why, and
/// stops: running on the defaults in its place would hide it, and a write would replace it.
pub fn execute(store: Arc<SqliteStore>, command: Command) -> Result<(), String> {
    if reads_settings(&command)
        && let Some(config) = mail_core::config::config_dir()
    {
        let found = mail_ui_model::damage::of_config(&config);
        if !found.is_empty() {
            let mut said: Vec<String> = found.iter().map(ToString::to_string).collect();
            said.push("fix or move the file aside, then run the command again".to_owned());
            return Err(said.join("\n"));
        }
    }
    // Sync needs an async runtime and the store by Arc, so it is dispatched here rather than
    // inside super::run, which is deliberately synchronous and testable.
    if matches!(command, Command::Sync) {
        let mail = mail_edge::mail(&store);
        let ends = mail_edge::block_on(mail.sync().run(Default::default()))
            .map_err(super::remedy::error)?;
        print!("{}", super::sync::run_text(&store, &ends));
        return Ok(());
    }
    if let Command::SyncFolder { account, path } = &command {
        print!("{}", super::sync_folder(store, account, path)?);
        return Ok(());
    }
    // Saving an attachment may have to download it first — a large IMAP message's attachments
    // stay on the server until asked for — and that needs the store by `Arc`, like sync.
    if let Command::Save {
        message,
        index,
        dir,
    } = &command
    {
        let mail = mail_edge::mail(&store);
        let download = |section: &mail_domain::Section| {
            mail_edge::block_on(mail.sync().fetch_part(*message, section))
        };
        let path = mail_core::attach::fetch_and_save(&store, *message, *index, dir, download)?;
        println!("{}", super::attach::saved(&path));
        return Ok(());
    }
    // The daemon and the clients that reach it. Dispatched here with `sync` and `watch` because
    // they need the store by `Arc`, which `super::run` has not.
    match &command {
        Command::Daemon { stop: false } => {
            let ended = mail_core::ipc::daemon::serve(
                store,
                std::sync::Arc::new(|store| {
                    let mail =
                        mail_edge::mail(&store).with_junk(mail_ui_model::settings::junk_policy());
                    match mail_edge::block_on(mail.sync().run(Default::default())) {
                        Ok(ends) => print!("{}", super::sync::run_text(&store, &ends)),
                        Err(why) => eprintln!("{}", super::remedy::error(why)),
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
            )
            .map_err(super::remedy::error)?;
            if ended == mail_core::ipc::daemon::Ended::Stopped {
                println!("stopped");
            }
            return Ok(());
        }
        Command::Daemon { stop: true } => {
            // Never starts one in order to stop it, which is why this is `connect` and not
            // `reach`: "there was nothing to stop" is a success, not a reason to spawn a daemon
            // and immediately ask it to leave.
            let Some(mut daemon) =
                mail_core::ipc::client::connect().map_err(|why| why.to_string())?
            else {
                println!("no daemon is running");
                return Ok(());
            };
            match daemon
                .ask(mail_core::ipc::wire::Request::Shutdown)
                .map_err(|why| why.to_string())?
            {
                mail_core::ipc::wire::Response::Stopping => println!("stopped"),
                other => return Err(answered(&other)),
            }
            return Ok(());
        }
        Command::Ping => {
            match mail_core::ipc::client::reach()
                .and_then(|mut d| d.ask(mail_core::ipc::wire::Request::Ping))
                .map_err(|why| why.to_string())?
            {
                pong @ mail_core::ipc::wire::Response::Pong { .. } => {
                    println!("{}", answered(&pong))
                }
                other => return Err(answered(&other)),
            }
            return Ok(());
        }
        _ => {}
    }

    // A Web Key Directory lookup needs the network: dispatched here with the other commands that
    // do, so `super::run` stays something a test can call without one.
    if let Command::Pgp(super::pgp::PgpCommand::Lookup { address }) = &command {
        let found = mail_edge::block_on(mail_edge::mail(&store).crypto().lookup_address(address))
            .map_err(|message| message.to_string())?;
        print!("{}", super::pgp::lookup(found.as_ref(), address));
        return Ok(());
    }
    // An encrypted message to someone with no key yet: their domain is asked, once, as the
    // draft is made, the one moment besides an explicit lookup that asks.
    if let Command::Compose {
        to, cc, openpgp, ..
    } = &command
        && openpgp.is_some_and(mail_domain::Mode::encrypts)
    {
        let addresses: Vec<String> = to.iter().chain(cc).map(|a| a.email.clone()).collect();
        let found = mail_edge::block_on(mail_edge::mail(&store).crypto().discover(&addresses));
        eprint!("{}", super::pgp::discovered(&found));
    }

    // Import and export print progress as they go, to stderr, and an upload needs the network:
    // dispatched here for the same reasons as `sync`.
    if let Command::Import { path, into } = &command {
        print!("{}", super::import::run(&store, path, into)?);
        return Ok(());
    }
    if let Command::Export { query, target } = &command {
        let said = super::export::run(&store, query, target).map_err(super::remedy::error)?;
        print!("{said}");
        return Ok(());
    }

    if let Command::Notify { set } = &command {
        let said = super::settings::notify(mail_ui_model::settings::person_root().as_ref(), *set)?;
        print!("{said}");
        return Ok(());
    }
    if let Command::Offline { address, set } = &command {
        let accounts = mail_core::sync::addresses(&store);
        let said = super::offline::command(
            mail_core::config::config_dir().as_deref(),
            store.as_ref(),
            &accounts,
            address.as_deref(),
            *set,
        )
        .map_err(super::remedy::error)?;
        print!("{said}");
        return Ok(());
    }
    // `watch` is `sync` that does not stop. It prints as it goes rather than at the end, because
    // "at the end" is when the user presses Ctrl-C.
    if let Command::Watch { notify, wait } = &command {
        // One watch per user: a second would announce every message twice. A unit that starts
        // while one runs by hand ends quietly (exit 0), so systemd does not restart it in a loop.
        // One told to wait (the login item's, which launchd would not start again after a clean
        // exit) asks again every [`AGAIN`] and takes over once that one ends, as at ⌘Q
        // of mailo.app. The settings are read once it watches, not before it waited.
        let watching = loop {
            match mail_core::ipc::watching::claim(mail_edge::profile().id()) {
                Ok(held) => break held,
                Err(mail_core::ipc::watching::Refused::AlreadyWatching) if *wait => {
                    std::thread::sleep(AGAIN);
                }
                Err(mail_core::ipc::watching::Refused::AlreadyWatching) => {
                    println!("a mailo watch is already running; leaving it to it");
                    return Ok(());
                }
                Err(mail_core::ipc::watching::Refused::Failed(why)) => {
                    return Err(format!(
                        "can't tell whether a mailo watch is running: {why}"
                    ));
                }
            }
        };
        if let Some(why) = watching.doorless() {
            eprintln!("Sync in an open window will not reach this watch: {why}");
        }
        // The unread count on the launcher, kept up whether or not a window is open. (Not on
        // macOS, where the Dock tile is mailo.app's, which keeps it itself.)
        if let Some(Err(e)) =
            crate::session::keep_the_launchers_badge(store.clone(), store.file().map(Into::into))
        {
            eprintln!("the launcher's unread count is off: {e}");
        }
        println!("watching. Ctrl-C to stop.");
        // Said as it happens, and flushed: a watch is read by someone waiting on it. An open
        // window sees what was stored from the store's file, not from here.
        let say = |watched: mail_core::sync::report::Watched| {
            use std::io::Write as _;
            print!("{}", super::sync::watched_text(&watched));
            if let mail_core::sync::report::Watched::Pass(end) = &watched {
                print!("{}", super::sync::sign_in_hint(&store, end));
            }
            let _ = std::io::stdout().flush();
        };
        let mail = mail_edge::mail(&store);
        // The desktop's notification service unless the watch was told to say nothing. What it
        // says, whose mail, in which mailboxes, with which sound, and how the junk filter
        // behaves are read as each pass ends or starts (`Notices`, `junk_policy`), so a change
        // in Settings reaches a watch that has been running for days. With no service to reach
        // it says nothing, and the watch goes on fetching.
        let notices = match notify {
            super::WatchNotify::Never => None,
            super::WatchNotify::AsSet => {
                let desktop = mail_os::desktop::connect();
                Some(mail_ui_model::settings::Notices::new(
                    Box::new(move |notification, sound| desktop.show_with(notification, sound)),
                    mail_ui_model::settings::person_root(),
                ))
            }
        };
        let notifier = notices
            .as_ref()
            .map(|notices| notices as &dyn mail_core::notify::Notifier);
        let whom = || {
            notices
                .as_ref()
                .map_or(mail_core::notify::Whom::Everyone, |notices| notices.whom())
        };
        // A Sync asked for at the watch's door (`ipc::watching`) is a pass now, here.
        let asked = Some(watching.asked());
        // Only ends when every account has stopped for a reason worth stopping for — a
        // credential the server refused, which no amount of retrying fixes.
        let ends = mail_edge::block_on(mail.sync().watch(
            notifier,
            mail_core::sync::watcher::Live {
                whom: &whom,
                junk: &mail_ui_model::settings::junk_policy,
            },
            &say,
            asked,
        ))
        .map_err(super::remedy::error)?;
        print!("{}", super::sync::run_text(&store, &ends));
        return Err("stopped watching; nothing left to watch".to_owned());
    }

    print!("{}", super::run(&mail_edge::mail(&store), &command)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::ipc::wire::Response;

    #[test]
    fn help_and_the_version_are_answered_before_any_store() {
        for command in [Command::Help, Command::Version] {
            assert!(matches!(prepare(Some(command)), Ok(Prepared::Finished)));
        }
    }

    #[test]
    fn the_daemon_s_answers_are_said_in_words() {
        // Not as Rust spells the wire's enum: `WrongVersion { daemon: 2, client: 3 }` is nothing
        // a person at a terminal reads.
        let cases = [
            (
                Response::Pong {
                    pid: 42,
                    version: "0.1.0".to_owned(),
                },
                "daemon 0.1.0 answering, pid 42",
            ),
            (
                Response::Refused("not now".to_owned()),
                "the daemon refused: not now",
            ),
            (
                Response::WrongVersion {
                    daemon: 2,
                    client: 3,
                },
                "speaks version 2 of its protocol and this mailo version 3",
            ),
        ];
        for (response, says) in cases {
            let said = answered(&response);
            assert!(said.contains(says), "{said}");
            assert!(!said.contains('{'), "{said}");
        }
    }
}
