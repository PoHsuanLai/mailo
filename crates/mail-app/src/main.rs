use mail_core::SqliteStore;

/// The one logger. What the libraries warn of (`log::warn!`) goes to stderr as a line, the way
/// they printed it before they logged: no level, no target, the text is the whole message. Which
/// records it writes is [`mail_app::edge::logged`]'s.
struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        mail_app::edge::logged(metadata.level(), metadata.target())
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
        log::set_max_level(log::LevelFilter::Info);
    }
    // `--profile` is the program's own, ahead of the verb: what follows is read as if it had not
    // been there.
    let (args, profile) = match mail_app::edge::take_profile_flag(std::env::args().skip(1)) {
        Ok(taken) => taken,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    // What the process was started with, read here and nowhere in the libraries: they are handed
    // it (`mail_app::edge`).
    let mut environment = mail_core::Environment::from_lookup(|name| std::env::var_os(name));
    if let Ok(exe) = std::env::current_exe() {
        environment.program = mail_core::Program::of_executable(&exe);
    }
    environment.hours = mail_app::edge::clock();
    mail_edge::install_environment(environment);
    // The OAuth clients it signs in as: porter's files and mailo's earlier one. Chosen here and
    // handed down, so no library and no test reads them for itself.
    mail_edge::install_clients(mail_core::OAuthClients::here());

    // No arguments opens the window, and `open <thread>` opens it on a conversation; anything
    // else is the CLI. One binary because they are one application over one store, and a
    // separate CLI would drift from what the UI does.
    // `mailo mailto:…`, as the desktop runs the scheme's handler: the window, on a composer
    // holding what the link asks for. The window makes the draft once it is open.
    // What the window is asked, which a second `mailo` would hand the window of its Profile that
    // is running, once the Profile is known (below).
    let start = match mail_ui::ui::ask::parse_args(&args) {
        Some(Ok(ask)) => Some(mail_ui::ui::ask::Open { ask, token: None }),
        Some(Err(message)) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
        None => None,
    };
    let command = if start.is_some() {
        None
    } else {
        match mail_cli::parse(&args) {
            Ok(command) => Some(command),
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(2);
            }
        }
    };

    let command = match mail_cli::exec::prepare(command) {
        Ok(mail_cli::exec::Prepared::Finished) => return,
        Ok(mail_cli::exec::Prepared::Run(command)) => command,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    let Some(roots) = roots() else {
        eprintln!(
            "cannot determine a data directory for this user (on Linux, set HOME or XDG_DATA_HOME)"
        );
        std::process::exit(1);
    };
    // Every folder mailo keeps is its owner's alone, a folder an earlier version made included.
    // One that cannot be narrowed is said, and the run goes on: the mail is still the person's.
    for dir in places_kept(&roots) {
        if let Err(e) = mail_core::config::make_private(&dir) {
            eprintln!("cannot make {} private: {e}", dir.display());
        }
    }
    // Whose mail this run is over: the default Profile's, which is the store as it was before
    // Profiles, unless `--profile` names another. A registry that cannot be read stops the run,
    // since going on over the default would hide the others.
    let registry = match mail_core::profile::Registry::load(&roots.data) {
        Ok(registry) => registry,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let chosen = match registry.resolve(profile.as_deref().unwrap_or("default")) {
        Ok(chosen) => chosen.clone(),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let dirs = Paths {
        db: chosen.db(&roots),
        blobs: chosen.blobs(&roots),
    };
    // A second `mailo` hands what it was asked to its Profile's window that is running and ends,
    // with the click's activation token: `mailo open <thread>` (a banner's click, or the person at
    // a terminal) opens the conversation there, `mailo open --inbox <account>` (a summary's click)
    // shows that account's inbox there, `mailo mailto:…` composes there, and a bare
    // `mailo` raises it, as macOS's launcher does for the app bundle. A window of another Profile
    // is never handed to. No window running (or no bus) is the ordinary start below.
    let token = std::env::var("XDG_ACTIVATION_TOKEN").ok();
    if let Some(start) = &start
        && mail_ui::handoff::deliver(chosen.id(), &*start.ask, token.as_deref())
    {
        return;
    }
    mail_edge::install_profile(chosen);
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
    // Plain drafts kept before every draft's text was flowed text are made so, once.
    if let Err(e) = mail_core::compose::reflow_stored(&store) {
        eprintln!("could not rewrite stored drafts as flowed text: {e}");
    }
    // OpenPGP keys kept before their dates were recorded have them read from their bytes, once.
    if let Err(e) = mail_runtime::pgp::date_keys(&store) {
        eprintln!("could not read the dates of stored OpenPGP keys: {e}");
    }

    let store = std::sync::Arc::new(store);
    // Every command is the cli's but `intents`: the steps that wait on the network or end the
    // process live in `mail_cli::exec`. No command is the window.
    if let Some(command) = command {
        let done = match command {
            // The one command that is the intents' and not the cli's: the process is where the
            // two meet.
            mail_cli::Command::Intents => mail_app::intents::serve(store),
            command => mail_cli::exec::execute(store, command),
        };
        if let Err(message) = done {
            eprintln!("{message}");
            std::process::exit(1);
        }
        return;
    }
    {
        let config = mail_core::config::config_dir();
        let look = mail_ui::ui::view::Appearance {
            marks: mail_ui::settings::person_root()
                .map(|root| mail_ui::settings::load(&root))
                .unwrap_or_default()
                .window
                .provider_marks
                .into(),
        };
        // What mailo wrote before quire: read only, for Spaces made before theme and motion
        // were theirs.
        let legacy = config
            .as_deref()
            .map(mail_ui::ui::appearance::legacy)
            .unwrap_or_default();
        // The look is the desktop's (`quire/appearance.toml`), which the window follows live.
        // What mailo kept for itself is brought over once, when the desktop has none, and
        // never written again.
        if let (Some(from), Some(to)) = (config.as_deref(), mail_ui::ui::appearance::desktop_dir())
        {
            mail_ui::ui::appearance::adopt(from, &to);
        }
        let ids = account_ids(&store);
        let dirs = config.and_then(|config| {
            mail_core::config::state_dir()
                .map(|state| mail_ui::ui::appearance::WindowDirs { config, state })
        });
        // The stored Spaces, or a first run over the accounts, written where the window
        // keeps them.
        let spaces = mail_ui::ui::space::boot(dirs.as_ref(), &ids, &legacy);
        if let Err(why) = mail_app::window::open(
            store,
            look,
            spaces,
            dirs,
            start.unwrap_or_else(mail_ui::ui::ask::Open::raise),
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

/// The directories Profiles' paths are under: mailo's data directory (`$XDG_DATA_HOME/mailo` on
/// Linux, the local application data folder on macOS and Windows; `mail_secrets::places`) and its
/// config directory.
/// The folders mailo keeps: the data and config roots, and the state and cache folders where they
/// are elsewhere (on Linux each place is its own folder).
fn places_kept(roots: &mail_core::profile::Roots) -> Vec<std::path::PathBuf> {
    let mut kept = vec![roots.data.clone(), roots.config.clone()];
    for path in [
        mail_core::config::state_dir(),
        mail_core::config::cache_dir(),
    ]
    .into_iter()
    .flatten()
    {
        if !kept.contains(&path) {
            kept.push(path);
        }
    }
    kept
}

fn roots() -> Option<mail_core::profile::Roots> {
    Some(mail_core::profile::Roots {
        data: mail_secrets::places::dir(mail_secrets::places::Place::Data)?,
        config: mail_core::config::config_dir()?,
    })
}
