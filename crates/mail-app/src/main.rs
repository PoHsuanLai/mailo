use mail_core::SqliteStore;

/// The one logger. What the libraries warn of (`log::warn!`) goes to stderr as a line, the way
/// they printed it before they logged: no level, no target, the text is the whole message.
struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{}", record.args());
        }
    }

    fn flush(&self) {}
}

fn main() {
    // Already set means a harness got there first; its logger stands.
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    // What the process was started with, read here and nowhere in the libraries: they are handed
    // it (`mail_app::edge`).
    let mut environment = mail_core::Environment::from_lookup(|name| std::env::var_os(name));
    if let Ok(exe) = std::env::current_exe() {
        environment.program = mail_core::Program::of_executable(&exe);
    }
    mail_app::edge::install_environment(environment);

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
        && mail_app::handoff::deliver(
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

    let command = match mail_app::cli::exec::prepare(command) {
        mail_app::cli::exec::Prepared::Finished => return,
        mail_app::cli::exec::Prepared::Run(command) => command,
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
        // set aside, untouched, before anything lists or syncs.
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
    // Every command is the cli's: the steps that wait on the network or end the process live in
    // `cli::exec`. No command is the window.
    if let Some(command) = command {
        mail_app::cli::exec::execute(store, command);
        return;
    }
    {
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
        if let (Some(from), Some(to)) = (config.as_deref(), mail_app::ui::appearance::desktop_dir())
        {
            mail_app::ui::appearance::adopt(from, &to);
        }
        let ids = account_ids(&store);
        let dirs = config.and_then(|config| {
            mail_core::config::state_dir()
                .map(|state| mail_app::ui::appearance::WindowDirs { config, state })
        });
        // The stored Spaces, or a first run over the accounts, written where the window
        // keeps them.
        let spaces = mail_app::ui::space::boot(dirs.as_ref(), &ids, &legacy);
        if let Err(why) = mail_app::ui::run(
            store,
            look,
            spaces,
            dirs,
            start.unwrap_or(mail_app::ui::Start::Inbox),
        ) {
            eprintln!("mailo: the window could not open: {why}");
            std::process::exit(1);
        }
    }
}

/// Account ids in the order they were added, for the first-run Spaces.
fn account_ids(store: &SqliteStore) -> Vec<porter_core::AccountId> {
    store
        .list_accounts()
        .unwrap_or_default()
        .into_iter()
        .map(|account| account.id)
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
