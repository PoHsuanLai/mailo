use mail_store::SqliteStore;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // No arguments opens the window, and `open <thread>` opens it on a conversation; anything
    // else is the CLI. One binary because they are one application over one store, and a
    // separate CLI would drift from what the UI does.
    // `mailo mailto:…`, as the desktop runs the scheme's handler: the window, on a composer
    // holding what the link asks for. The draft is made once the store is open, below.
    let mailto = mail_app::ui::mailto_of(&args);
    let start = match mail_app::ui::start_of(&args) {
        _ if mailto.is_some() => Some(mail_app::ui::Start::Inbox),
        Some(Ok(start)) => Some(start),
        Some(Err(message)) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
        None => None,
    };
    // `mailo open <thread>` while a window is running hands the conversation to it and ends: a
    // banner's click, or the person at a terminal, opens it where they are already reading. The
    // click's activation token travels with it. No window running (or no bus) is the ordinary
    // start below.
    if let Some(mail_app::ui::Start::Thread(thread)) = &start
        && mailto.is_none()
        && mail_app::ui::handoff::deliver(
            *thread,
            std::env::var("XDG_ACTIVATION_TOKEN").ok().as_deref(),
        )
    {
        return;
    }
    let command = if start.is_some() {
        None
    } else {
        match mail_app::cli::parse(&args) {
            Ok(command) => Some(command),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(2);
            }
        }
    };

    // The schema needs nothing but somewhere to write it: no store, no config directory.
    if let Some(mail_app::cli::Command::WriteSchema { dir }) = &command {
        match mail_app::settings::write_schema(dir) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }

    // Discovery needs the network and, unless `--yes` was given, a person at a terminal to say
    // yes to what it found; both are here rather than in `cli::run`, which is synchronous and
    // tested without either. Nothing is stored and nothing is sent to a found server before
    // the yes.
    if let Some(mail_app::cli::Command::AccountDiscover { address }) = &command {
        match mail_app::cli::discover::show(address, |address| {
            mail_core::discover::lookup(address, chrono::Utc::now())
        }) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    let command = match command {
        Some(command) => match mail_app::cli::discover::before_add(
            command,
            |address| mail_core::discover::lookup(address, chrono::Utc::now()),
            mail_app::cli::discover::Terminal::of_stdin(),
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
        Some(command) => match mail_app::cli::discover::before_add_jmap(
            command,
            mail_core::discover::find_jmap,
            mail_app::cli::discover::Terminal::of_stdin(),
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
        Some(mail_app::cli::Command::Reply {
            message,
            scope,
            body: _,
        }) => {
            let mut body = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body) {
                eprintln!("cannot read the message body: {e}");
                std::process::exit(1);
            }
            Some(mail_app::cli::Command::Reply {
                message,
                scope,
                body,
            })
        }
        Some(mail_app::cli::Command::Compose {
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
            Some(mail_app::cli::Command::Compose {
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
        Some(mail_app::cli::Command::Forward {
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
            Some(mail_app::cli::Command::Forward {
                message,
                to,
                body,
                carry,
            })
        }
        // `signature` takes its text from stdin too, unless it is being cleared.
        Some(mail_app::cli::Command::Signature {
            address,
            clear: false,
            text: _,
        }) => {
            let mut text = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut text) {
                eprintln!("cannot read the signature: {e}");
                std::process::exit(1);
            }
            Some(mail_app::cli::Command::Signature {
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
        Some(mail_app::cli::Command::Print {
            target,
            out: mail_app::cli::PrintTo::Unsaid,
            pages,
        }) => {
            use std::io::IsTerminal as _;
            let out = if std::io::stdout().is_terminal() {
                mail_app::cli::PrintTo::Into(std::path::PathBuf::from("."))
            } else {
                mail_app::cli::PrintTo::Stdout
            };
            Some(mail_app::cli::Command::Print { target, out, pages })
        }
        other => other,
    };

    let Some(dirs) = paths() else {
        eprintln!(
            "cannot determine a data directory for this user (on Linux, set HOME or XDG_DATA_HOME)"
        );
        std::process::exit(1);
    };
    if let Err(e) = std::fs::create_dir_all(&dirs.blobs) {
        eprintln!("cannot create {}: {e}", dirs.blobs.display());
        std::process::exit(1);
    }

    let store = match SqliteStore::open(&dirs.db, &dirs.blobs) {
        Ok(store) => store,
        Err(e) => {
            eprintln!("cannot open {}: {e}", dirs.db.display());
            std::process::exit(1);
        }
    };

    // Messages an older parser misread are re-read from their stored bytes, once, here — the
    // one place every command and the window pass through. Usually an empty queue and one
    // query. A failure is reported and does not stop the command: the mail is still readable.
    if let Err(e) = mail_runtime::reparse_queued(&store) {
        eprintln!("could not re-read stored headers: {e}");
    }
    // OpenPGP keys kept before their dates were recorded have them read from their bytes, once.
    if let Err(e) = mail_runtime::pgp::date_keys(&store) {
        eprintln!("could not read the dates of stored OpenPGP keys: {e}");
    }

    let store = std::sync::Arc::new(store);
    // The link to the desktop's accountd, decided once, here: the window, `watch` and every command
    // that signs in to a server go through it (`mail_runtime::platform_secrets`). `watch` is
    // nobody's foreground, so it asks as a background use. Only the window and `watch` say which
    // was chosen: a command's output is its own.
    {
        use mail_app::cli::Command;
        let background = matches!(command, Some(Command::Watch { .. }));
        let linked = mail_app::accountd::start(if background {
            porter_core::consent::Usage::Background
        } else {
            porter_core::consent::Usage::Interactive
        });
        let says = background || command.is_none();
        if says {
            eprintln!("mailo: accounts link: {}", linked.name());
        }
        // Linked to accountd, only its accounts are Mail's: the ones Mail signed in itself are
        // set aside, untouched, before anything lists, syncs or adopts.
        mail_app::accountd::hold_back(&store, &linked);
        // The accounts accountd offers, read into the store before anything draws or syncs.
        match mail_app::accountd::read(&store, &linked) {
            Ok(Some(read)) => {
                if let Some(line) = mail_app::accountd::said(&read).filter(|_| says) {
                    eprintln!("mailo: {line}");
                }
            }
            Ok(None) => {}
            Err(why) => eprintln!("mailo: reading the desktop's accounts: {why}"),
        }
    }
    // After the link is chosen, because linked to accountd the accounts Mail signed in itself are
    // not adopted (their keyring items stay as they are). The window and `watch` live long enough
    // to move what an earlier build kept in the keyring into porter's store: once, on a thread of
    // its own, and nothing waits on it (until it has run the old entries are read behind the new
    // store). A failure is logged, not fatal.
    mail_app::adoption::for_command(command.as_ref(), &store, mail_app::adoption::platform);
    let start = match &mailto {
        Some(link) => match mail_app::ui::start_mailto(&store, link, chrono::Utc::now()) {
            Ok(compose) => Some(compose),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        },
        None => start,
    };
    // Sync needs an async runtime and the store by Arc, so it is dispatched here rather than
    // inside mail_app::cli::run, which is deliberately synchronous and testable.
    if matches!(command, Some(mail_app::cli::Command::Sync)) {
        match mail_core::sync::run(store.clone(), chrono::Utc::now(), Default::default()) {
            Ok(ends) => print!("{}", mail_app::cli::sync::run_text(&store, &ends)),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(mail_app::cli::Command::SyncFolder { account, path }) = &command {
        match mail_app::cli::sync_folder(store, account, path, chrono::Utc::now()) {
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
    if let Some(mail_app::cli::Command::Save {
        message,
        index,
        dir,
    }) = &command
    {
        let download = |section: &str| {
            mail_core::sync::fetch_part(&store, *message, section, chrono::Utc::now())
        };
        match mail_core::attach::fetch_and_save(&store, *message, *index, dir, download) {
            Ok(said) => println!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    // The daemon and the clients that reach it. Dispatched here with `sync` and `watch` because
    // they need the store by `Arc` and an exit code, neither of which `mail_app::cli::run` has.
    match &command {
        Some(mail_app::cli::Command::Daemon { stop: false }) => {
            match mail_core::ipc::daemon::serve(
                store,
                std::sync::Arc::new(|store| {
                    match mail_core::sync::run(
                        store.clone(),
                        chrono::Utc::now(),
                        Default::default(),
                    ) {
                        Ok(ends) => {
                            print!("{}", mail_app::cli::sync::run_text(&store, &ends));
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
            ) {
                Ok(out) => {
                    print!("{out}");
                    return;
                }
                Err(message) => {
                    eprintln!("{message}");
                    std::process::exit(1);
                }
            }
        }
        Some(mail_app::cli::Command::Daemon { stop: true }) => {
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
        Some(mail_app::cli::Command::Ping) => {
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
    // do, so `mail_app::cli::run` stays something a test can call without one.
    if let Some(mail_app::cli::Command::Pgp(mail_core::pgp::PgpCommand::Lookup { address })) =
        &command
    {
        match mail_core::pgp::lookup(&store, address, chrono::Utc::now()) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    // An encrypted message to someone with no key yet: their domain is asked, once, as the
    // draft is made — the other moment the brief names besides an explicit lookup.
    if let Some(mail_app::cli::Command::Compose {
        to, cc, openpgp, ..
    }) = &command
        && openpgp.encrypts()
    {
        let addresses: Vec<String> = to.iter().chain(cc).map(|a| a.email.clone()).collect();
        let said = mail_core::pgp::discover(&store, &addresses, chrono::Utc::now());
        eprint!("{said}");
    }

    // Import and export print progress as they go, to stderr, and an upload needs the network:
    // dispatched here for the same reasons as `sync`.
    if let Some(mail_app::cli::Command::Import { path, into }) = &command {
        match import(&store, path, into) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(mail_app::cli::Command::Export { query, target }) = &command {
        let now = chrono::Utc::now();
        let exported = mail_core::export::select(&store, query, now).and_then(|chosen| {
            eprintln!("{} message(s) match", chosen.len());
            mail_core::export::export(&store, &chosen, target, now, &mut |done| {
                eprintln!("  {} written", done.written);
            })
        });
        match exported {
            Ok(done) => print!("{}", mail_core::export::said(&done, target)),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }

    // `watch` is `sync` that does not stop. It prints as it goes rather than at the end, because
    // "at the end" is when the user presses Ctrl-C.
    if let Some(mail_app::cli::Command::Notify { set }) = &command {
        match mail_app::cli::settings::notify(mail_app::settings::person_root().as_ref(), *set) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(mail_app::cli::Command::Offline { address, set }) = &command {
        let accounts = mail_core::sync::addresses(&store);
        match mail_core::offline::command(
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
    if let Some(mail_app::cli::Command::Intents) = &command {
        // What the router starts by D-Bus activation: no window, the same store. A second one
        // finds the name taken and ends quietly (exit 0), so a start that races another is not
        // a failure.
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let provider = mail_app::intents::Provider::new(
                store.clone(),
                std::sync::Arc::new(mail_runtime::KeyringSigningStore::default()),
                mail_app::intents::Opener::window(),
            );
            match mail_app::intents::serve(provider) {
                Ok(()) | Err(mail_app::intents::ServeError::Taken) => return,
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
    if let Some(mail_app::cli::Command::Watch { notify }) = &command {
        let notifications = match notify {
            mail_app::cli::WatchNotify::Never => mail_core::notify::Setting::Off,
            mail_app::cli::WatchNotify::AsSet => mail_core::notify::Setting::from(
                mail_app::settings::person_root()
                    .map(|root| mail_app::settings::load(&root))
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
        if let Some(launcher) = mail_app::ui::launcher::platform()
            && let Err(e) = mail_app::session::keep_the_badge(
                store.clone(),
                launcher.0,
                mail_app::session::EVERY,
            )
        {
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
            print!("{}", mail_app::cli::sync::watched_text(&watched));
            let _ = std::io::stdout().flush();
        };
        match mail_core::sync::watch(store.clone(), chrono::Utc::now(), notifications, &say) {
            // Only reached when every account has stopped for a reason worth stopping for — a
            // credential the server refused, which no amount of retrying fixes.
            Ok(ends) => {
                print!("{}", mail_app::cli::sync::run_text(&store, &ends));
                eprintln!("stopped watching; nothing left to watch");
                std::process::exit(1);
            }
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
    }

    match command {
        Some(command) => match mail_app::cli::run(&store, &command, chrono::Utc::now()) {
            Ok(output) => print!("{output}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        },
        None => {
            let config = mail_core::config::config_dir();
            let look = mail_app::ui::view::Appearance {
                marks: mail_app::settings::person_root()
                    .map(|root| mail_app::settings::load(&root))
                    .unwrap_or_default()
                    .window
                    .provider_marks
                    .into(),
            };
            // What mailo wrote before quire: read only, for Spaces made before theme and motion
            // were theirs.
            let legacy = config
                .as_deref()
                .map(mail_app::ui::appearance::legacy)
                .unwrap_or_default();
            // The look is the desktop's (`quire/appearance.toml`), which the window follows live.
            // What mailo kept for itself is brought over once, when the desktop has none, and
            // never written again.
            if let (Some(from), Some(to)) =
                (config.as_deref(), mail_app::ui::appearance::desktop_dir())
            {
                mail_app::ui::appearance::adopt(from, &to);
            }
            let ids = account_ids(&store);
            let spaces = match &config {
                Some(dir) => {
                    let loaded = mail_app::ui::space::load(dir);
                    if loaded.spaces.is_empty() {
                        let mut made = mail_app::ui::space::first_run(&ids);
                        mail_app::ui::space::inherit(&mut made, &legacy);
                        let _ = mail_app::ui::space::save(dir, &made);
                        made
                    } else {
                        loaded
                    }
                }
                None => {
                    let mut made = mail_app::ui::space::first_run(&ids);
                    mail_app::ui::space::inherit(&mut made, &legacy);
                    made
                }
            };
            let dirs = config.and_then(|config| {
                mail_core::config::state_dir()
                    .map(|state| mail_app::ui::appearance::WindowDirs { config, state })
            });
            mail_app::ui::run(
                store,
                look,
                spaces,
                dirs,
                start.unwrap_or(mail_app::ui::Start::Inbox),
            );
        }
    }
}

/// `mailo import`: keep the mail here, or queue it for a mailbox and send it now.
fn import(
    store: &std::sync::Arc<SqliteStore>,
    path: &std::path::Path,
    into: &mail_core::import::Destination,
) -> Result<String, String> {
    use mail_core::import::{self, Destination};
    let now = chrono::Utc::now();
    let source = import::detect(path)?;
    let mut progress = |so_far: &import::Imported| {
        eprintln!("  {} read, {} new", so_far.read, so_far.added);
    };
    match into {
        Destination::Local => {
            let total = import::into_local(store, &source, now, &mut progress)?;
            Ok(import::said(&total, into))
        }
        Destination::Mailbox { account, folder } => {
            let (id, total) =
                import::queue_uploads(store, account, folder, &source, now, &mut progress)?;
            let mut out = import::said(&total, into);
            // Sent now, so the user sees it go; whatever fails stays queued for the next sync.
            let report = mail_core::sync::drain(store, id, now)?;
            out.push_str(&format!("{} uploaded\n", report.appended));
            if report.still_queued > 0 {
                out.push_str(&format!(
                    "{} still queued; `mailo sync` tries again\n",
                    report.still_queued
                ));
            }
            for note in &report.needs_attention {
                out.push_str(&format!("  needs attention: {note}\n"));
            }
            Ok(out)
        }
    }
}

/// Account ids in the order they were added, for the first-run Spaces.
fn account_ids(store: &SqliteStore) -> Vec<porter_core::AccountId> {
    let db = store.connection();
    let Ok(mut stmt) = db.prepare(&format!(
        "SELECT id FROM {} ORDER BY created_at",
        store.accounts()
    )) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.filter_map(|row| row.ok())
        .filter_map(|id| id.parse().ok())
        .map(mail_domain::id::account_id_from_uuid)
        .collect()
}

struct Paths {
    db: std::path::PathBuf,
    blobs: std::path::PathBuf,
}

/// Where the database and blobs live: mailo's data directory (`$XDG_DATA_HOME/mailo` on Linux,
/// the local application data folder on macOS and Windows; `mail_runtime::places`).
fn paths() -> Option<Paths> {
    let base = mail_runtime::places::dir(mail_runtime::places::Place::Data)?;
    Some(Paths {
        db: base.join("mail.db"),
        blobs: base.join("blobs"),
    })
}
