//! What only the program decides from its command line and its desktop: the profile flag, the
//! hours the person keeps, and which log records reach stderr. The runtime, the environment and
//! the Profile `main` records are `mail-edge`'s, below the command line and the intents provider
//! as well as `main`.

use mail_core::when::Clock;
use mail_ui::ui::TRACE;

/// The profile named by a leading `--profile <x>` or `--profile=<x>`, and the arguments without
/// it, so everything that reads the command line afterwards reads what it always read.
///
/// The flag is the program's own and goes ahead of the verb (`mailo --profile work open <thread>`,
/// `mailo --profile work mailto:…`, `mailo --profile work`); after the verb the words are the
/// verb's. A flag with no value is an error, and given twice the last one counts.
pub fn take_profile_flag(
    args: impl IntoIterator<Item = String>,
) -> Result<(Vec<String>, Option<String>), String> {
    let mut rest = args.into_iter().peekable();
    let mut profile = None;
    loop {
        let value = match rest.peek().map(String::as_str) {
            Some("--profile") => {
                rest.next();
                rest.next()
            }
            Some(arg) if arg.starts_with("--profile=") => rest
                .next()
                .and_then(|arg| arg.strip_prefix("--profile=").map(str::to_owned)),
            _ => break,
        };
        match value.filter(|value| !value.is_empty()) {
            Some(value) => profile = Some(value),
            None => {
                return Err(
                    "--profile needs a profile id or name: mailo --profile <id-or-name> <command>"
                        .to_owned(),
                );
            }
        }
    }
    Ok((rest.collect(), profile))
}

/// How the person keeps the hours of the day, for what the command line and `mailo intents`
/// write (`main` hands it down as `Environment::hours`): the desktop's one `[region] hours`
/// setting in `quire/appearance.toml`, Automatic settled by the session's locale.
///
/// Read once, as the process starts. The window does not ask here: it follows the same setting
/// live, through the desktop settings it already watches (`ui::clock::hours`).
pub fn clock() -> Clock {
    // The desktop's store, as the window's `DesktopSettings` opens it.
    let store = ds_settings::Store::new(ds_settings::ConfigRoot::Xdg, ds_settings::AppName::QUIRE);
    let hours = store
        .load::<ds_settings::AppearanceFile>()
        .value
        .region
        .hours;
    mail_ui::ui::clock_from(hours.resolve(mail_ui::ui::time_locale()))
}

/// Libraries whose warnings are about the input they were handed, not about a fault: an HTML
/// parser warns of malformed markup, which is what most mail is, and nothing anyone can act on.
const INPUT_PARSERS: &[&str] = &["html5ever", "markup5ever", "xml5ever"];

/// Whether the one logger (`main`'s) writes a record at `level` from `target` to stderr: the
/// warnings and errors, but not an HTML parser's, and the window's [`TRACE`]s at info.
pub fn logged(level: log::Level, target: &str) -> bool {
    if target == TRACE {
        return level <= log::Level::Info;
    }
    let root = target.split("::").next().unwrap_or_default();
    level <= log::Level::Warn && !INPUT_PARSERS.contains(&root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taken(words: &[&str]) -> Result<(Vec<String>, Option<String>), String> {
        take_profile_flag(words.iter().map(|word| (*word).to_owned()))
    }

    #[test]
    fn the_window_traces_reach_stderr_and_other_info_does_not() {
        assert!(logged(log::Level::Info, TRACE));
        assert!(!logged(log::Level::Debug, TRACE));
        assert!(!logged(log::Level::Info, "mail_ui::ui::handoff"));
        assert!(logged(log::Level::Warn, "mail_ui::ui::handoff"));
        assert!(!logged(log::Level::Warn, "html5ever::tree_builder"));
    }

    #[test]
    fn the_profile_flag_is_taken_ahead_of_the_verb_and_nothing_else_moves() {
        const CASES: &[(&[&str], &[&str], Option<&str>)] = &[
            (&[], &[], None),
            (&["list"], &["list"], None),
            (&["--profile", "work"], &[], Some("work")),
            (&["--profile=work"], &[], Some("work")),
            (
                &["--profile", "work", "list", "inbox"],
                &["list", "inbox"],
                Some("work"),
            ),
            (
                &["--profile=work", "open", "t1"],
                &["open", "t1"],
                Some("work"),
            ),
            (
                &["--profile", "work", "mailto:a@example.test"],
                &["mailto:a@example.test"],
                Some("work"),
            ),
            // Twice: the last one.
            (
                &["--profile", "a", "--profile=b", "sync"],
                &["sync"],
                Some("b"),
            ),
            // After the verb it is the verb's, which says what it makes of it.
            (
                &["list", "--profile", "work"],
                &["list", "--profile", "work"],
                None,
            ),
            (&["search", "--profile=x"], &["search", "--profile=x"], None),
            // A word that merely starts like it is not it.
            (&["--profiles", "x"], &["--profiles", "x"], None),
        ];
        for (words, rest, profile) in CASES {
            let (got_rest, got) = taken(words).unwrap();
            assert_eq!(&got_rest, rest, "{words:?}");
            assert_eq!(got.as_deref(), *profile, "{words:?}");
        }
    }

    #[test]
    fn a_profile_flag_with_no_value_is_refused() {
        for words in [
            &["--profile"][..],
            &["--profile="],
            &["--profile", "", "list"],
        ] {
            let error = taken(words).unwrap_err();
            assert!(error.contains("--profile needs"), "{words:?}: {error}");
        }
    }

    #[test]
    fn the_desktop_setting_and_the_locale_say_which_clock_the_person_keeps() {
        use ds_settings::{HourCycle, time_locale};
        const AUTO: HourCycle = HourCycle::Auto;
        type Case = (
            HourCycle,
            Option<&'static str>,
            Option<&'static str>,
            Option<&'static str>,
            Clock,
        );
        const CASES: &[Case] = &[
            (AUTO, None, None, Some("en_US.UTF-8"), Clock::H12),
            (AUTO, None, None, Some("de_DE.UTF-8"), Clock::H24),
            // French Canada writes twenty-four hours though Canada is a twelve-hour country.
            (AUTO, None, None, Some("fr_CA.UTF-8"), Clock::H24),
            (AUTO, None, None, Some("C"), Clock::H24),
            (AUTO, None, None, None, Clock::H24),
            // LC_ALL beats LC_TIME, which beats LANG; an empty one is not set.
            (
                AUTO,
                Some("de_DE.UTF-8"),
                Some("en_US.UTF-8"),
                Some("en_US.UTF-8"),
                Clock::H24,
            ),
            (
                AUTO,
                None,
                Some("en_US.UTF-8"),
                Some("de_DE.UTF-8"),
                Clock::H12,
            ),
            (AUTO, Some(""), Some(""), Some("en_US.UTF-8"), Clock::H12),
            // The person's choice in the desktop's settings beats the locale.
            (HourCycle::H24, None, None, Some("en_US.UTF-8"), Clock::H24),
            (HourCycle::H12, None, None, Some("de_DE.UTF-8"), Clock::H12),
        ];
        for (hours, all, time, lang, want) in CASES {
            let locale = time_locale(*all, *time, *lang);
            assert_eq!(
                mail_ui::ui::clock_from(hours.resolve(locale)),
                *want,
                "{hours:?} {all:?} {time:?} {lang:?}"
            );
        }
    }
}
