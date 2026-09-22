//! `mailo` — the command line. The Dioxus shell will call the same store methods.

mod account;
mod attach;
mod cli;
mod compose;
mod query;
mod reader;
mod snooze;
mod sync;
mod ui;
mod view;

use mail_store::SqliteStore;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // No arguments opens the window; anything else is the CLI. One binary because they are one
    // application over one store, and a separate CLI would drift from what the UI does.
    let command = if args.is_empty() {
        None
    } else {
        match cli::parse(&args) {
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
        Some(cli::Command::Reply {
            message,
            scope,
            body: _,
        }) => {
            let mut body = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body) {
                eprintln!("cannot read the message body: {e}");
                std::process::exit(1);
            }
            Some(cli::Command::Reply {
                message,
                scope,
                body,
            })
        }
        Some(cli::Command::Forward {
            message,
            to,
            body: _,
        }) => {
            let mut body = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut body) {
                eprintln!("cannot read the covering note: {e}");
                std::process::exit(1);
            }
            Some(cli::Command::Forward { message, to, body })
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
    // inside cli::run, which is deliberately synchronous and testable.
    if matches!(command, Some(cli::Command::Sync)) {
        match sync::run(store, chrono::Utc::now()) {
            Ok(output) => print!("{output}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
        return;
    }

    match command {
        Some(command) => match cli::run(&store, &command, chrono::Utc::now()) {
            Ok(output) => print!("{output}"),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        },
        None => ui::run(store),
    }
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
