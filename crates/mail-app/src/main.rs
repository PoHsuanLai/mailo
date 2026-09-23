use mail_store::SqliteStore;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // No arguments opens the window; anything else is the CLI. One binary because they are one
    // application over one store, and a separate CLI would drift from what the UI does.
    let command = if args.is_empty() {
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

    // `watch` is `sync` that does not stop. It prints as it goes rather than at the end, because
    // "at the end" is when the user presses Ctrl-C.
    if matches!(command, Some(mail_app::cli::Command::Watch)) {
        println!("watching. Ctrl-C to stop.");
        match mail_app::sync::watch(store, chrono::Utc::now()) {
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
            mail_app::ui::run(store, look, spaces, dirs);
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
