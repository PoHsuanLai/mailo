use mail_store::SqliteStore;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // No arguments opens the window, and `open <thread>` opens it on a conversation; anything
    // else is the CLI. One binary because they are one application over one store, and a
    // separate CLI would drift from what the UI does.
    let start = match mail_app::ui::start_of(&args) {
        Some(Ok(start)) => Some(start),
        Some(Err(message)) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
        None => None,
    };
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

    // Discovery needs the network and, unless `--yes` was given, a person at a terminal to say
    // yes to what it found; both are here rather than in `cli::run`, which is synchronous and
    // tested without either. Nothing is stored and nothing is sent to a found server before
    // the yes.
    if let Some(mail_app::cli::Command::AccountDiscover { address }) = &command {
        match mail_app::discover::show(address, |address| {
            mail_app::discover::lookup(address, chrono::Utc::now())
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
        Some(command) => match mail_app::discover::before_add(
            command,
            |address| mail_app::discover::lookup(address, chrono::Utc::now()),
            mail_app::discover::Terminal::of_stdin(),
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
            })
        }
        Some(mail_app::cli::Command::Forward {
            message,
            to,
            body: _,
        }) => {
            let mut body = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body) {
                eprintln!("cannot read the covering note: {e}");
                std::process::exit(1);
            }
            Some(mail_app::cli::Command::Forward { message, to, body })
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
        eprintln!("cannot determine a data directory; set HOME or XDG_DATA_HOME");
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

    let store = std::sync::Arc::new(store);
    // Sync needs an async runtime and the store by Arc, so it is dispatched here rather than
    // inside mail_app::cli::run, which is deliberately synchronous and testable.
    if matches!(command, Some(mail_app::cli::Command::Sync)) {
        match mail_app::sync::run(store, chrono::Utc::now()) {
            Ok(ran) => print!("{}", ran.text),
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
            mail_app::sync::fetch_part(&store, *message, section, chrono::Utc::now())
        };
        match mail_app::attach::fetch_and_save(&store, *message, *index, dir, download) {
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
        Some(mail_app::cli::Command::Daemon { stop: false }) => match mail_app::ipc::daemon::serve(
            store,
            std::sync::Arc::new(
                |store| match mail_app::sync::run(store, chrono::Utc::now()) {
                    Ok(ran) => print!("{}", ran.text),
                    Err(why) => eprintln!("{why}"),
                },
            ),
        ) {
            Ok(out) => {
                print!("{out}");
                return;
            }
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        },
        Some(mail_app::cli::Command::Daemon { stop: true }) => {
            // Never starts one in order to stop it, which is why this is `connect` and not
            // `reach`: "there was nothing to stop" is a success, not a reason to spawn a daemon
            // and immediately ask it to leave.
            match mail_app::ipc::client::connect() {
                Ok(None) => println!("no daemon is running"),
                Ok(Some(mut daemon)) => match daemon.ask(mail_app::ipc::wire::Request::Shutdown) {
                    Ok(mail_app::ipc::wire::Response::Stopping) => println!("stopped"),
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
            match mail_app::ipc::client::reach()
                .and_then(|mut d| d.ask(mail_app::ipc::wire::Request::Ping))
            {
                Ok(mail_app::ipc::wire::Response::Pong { pid, version }) => {
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
        let exported = mail_app::export::select(&store, query, now).and_then(|chosen| {
            eprintln!("{} message(s) match", chosen.len());
            mail_app::export::export(&store, &chosen, target, now, &mut |done| {
                eprintln!("  {} written", done.written);
            })
        });
        match exported {
            Ok(done) => print!("{}", mail_app::export::said(&done, target)),
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
        match mail_app::notify::command(mail_app::appearance::config_dir().as_deref(), *set) {
            Ok(said) => print!("{said}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(mail_app::cli::Command::Watch { notify }) = &command {
        let notifications = match notify {
            mail_app::cli::WatchNotify::Never => mail_app::notify::Setting::Off,
            mail_app::cli::WatchNotify::AsSet => mail_app::appearance::config_dir()
                .as_deref()
                .map(mail_app::notify::load)
                .unwrap_or_default(),
        };
        println!("watching. Ctrl-C to stop.");
        match mail_app::sync::watch(store, chrono::Utc::now(), notifications) {
            // Only reached when every account has stopped for a reason worth stopping for — a
            // credential the server refused, which no amount of retrying fixes.
            Ok(ran) => {
                print!("{}", ran.text);
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
            let config = mail_app::appearance::config_dir();
            let look = config
                .as_deref()
                .map(mail_app::appearance::load)
                .unwrap_or_default();
            let ids = account_ids(&store);
            let spaces = match &config {
                Some(dir) => {
                    let loaded = mail_app::space::load(dir);
                    if loaded.spaces.is_empty() {
                        let mut made = mail_app::space::first_run(&ids);
                        mail_app::space::inherit(&mut made, &look);
                        let _ = mail_app::space::save(dir, &made);
                        made
                    } else {
                        loaded
                    }
                }
                None => {
                    let mut made = mail_app::space::first_run(&ids);
                    mail_app::space::inherit(&mut made, &look);
                    made
                }
            };
            let dirs = config.and_then(|config| {
                mail_app::appearance::state_dir()
                    .map(|state| mail_app::appearance::WindowDirs { config, state })
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
    into: &mail_app::import::Destination,
) -> Result<String, String> {
    use mail_app::import::{self, Destination};
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
            let report = mail_app::sync::drain(store, id, now)?;
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
fn account_ids(store: &SqliteStore) -> Vec<mail_domain::AccountId> {
    let db = store.connection();
    let Ok(mut stmt) = db.prepare("SELECT id FROM accounts ORDER BY created_at") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.filter_map(|row| row.ok())
        .filter_map(|id| id.parse().ok())
        .map(mail_domain::AccountId::from_uuid)
        .collect()
}

struct Paths {
    db: std::path::PathBuf,
    blobs: std::path::PathBuf,
}

/// Where the database and blobs live, following the XDG base directory spec.
fn paths() -> Option<Paths> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/share"))
        })?
        .join("mailo");
    Some(Paths {
        db: base.join("mail.db"),
        blobs: base.join("blobs"),
    })
}
