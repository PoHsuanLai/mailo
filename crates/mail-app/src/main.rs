//! `mailo` — the command line. The Dioxus shell will call the same store methods.

mod cli;
mod view;

use mail_store::SqliteStore;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let command = match cli::parse(&args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
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

    match cli::run(&store, &command, chrono::Utc::now()) {
        Ok(output) => print!("{output}"),
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
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
