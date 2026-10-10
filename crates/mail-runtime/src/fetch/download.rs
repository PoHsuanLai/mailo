//! An attachment being saved to disk.

use mail_domain::Retry;
use std::path::PathBuf;

/// Where one attachment download stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Download {
    Idle,
    /// Bytes are arriving. `of` is `None` when the server did not say how many there will be.
    Running {
        done: u64,
        of: Option<u64>,
    },
    /// Written here.
    Saved(PathBuf),
    /// It did not finish, with what to do about it.
    Failed {
        retry: Retry,
        why: String,
    },
}

/// What happened to the download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadEvent {
    Start,
    /// Bytes so far, and the total when known.
    Progress(u64, Option<u64>),
    Saved(PathBuf),
    Failed {
        retry: Retry,
        why: String,
    },
    /// The person has seen the outcome.
    Dismiss,
}

/// What the caller must do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadEffect {
    Begin,
}

impl Download {
    /// What `event` does to the download.
    pub fn step(&self, event: DownloadEvent) -> (Download, Option<DownloadEffect>) {
        match (self, event) {
            (Download::Idle | Download::Failed { .. }, DownloadEvent::Start) => (
                Download::Running { done: 0, of: None },
                Some(DownloadEffect::Begin),
            ),
            (Download::Running { .. }, DownloadEvent::Progress(done, of)) => {
                // A total smaller than what has arrived is a server that guessed wrong; the bytes
                // are the fact.
                let of = of.map(|total| total.max(done));
                (Download::Running { done, of }, None)
            }
            (Download::Running { .. }, DownloadEvent::Saved(path)) => (Download::Saved(path), None),
            (Download::Running { .. }, DownloadEvent::Failed { retry, why }) => {
                (Download::Failed { retry, why }, None)
            }
            (Download::Saved(_) | Download::Failed { .. }, DownloadEvent::Dismiss) => {
                (Download::Idle, None)
            }
            _ => (self.clone(), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed() -> Download {
        Download::Failed {
            retry: Retry::Now,
            why: "dropped".into(),
        }
    }

    fn running() -> Download {
        Download::Running { done: 0, of: None }
    }

    #[test]
    fn start_begins_from_idle_or_failed() {
        for from in [Download::Idle, failed()] {
            assert_eq!(
                from.step(DownloadEvent::Start),
                (running(), Some(DownloadEffect::Begin))
            );
        }
    }

    #[test]
    fn start_while_running_or_saved_does_nothing() {
        for from in [
            Download::Running {
                done: 5,
                of: Some(9),
            },
            Download::Saved("a".into()),
        ] {
            assert_eq!(from.step(DownloadEvent::Start), (from.clone(), None));
        }
    }

    #[test]
    fn progress_updates_and_never_exceeds_the_total() {
        let (to, _) = running().step(DownloadEvent::Progress(10, Some(100)));
        assert_eq!(
            to,
            Download::Running {
                done: 10,
                of: Some(100)
            }
        );
        let (to, _) = to.step(DownloadEvent::Progress(120, Some(100)));
        assert_eq!(
            to,
            Download::Running {
                done: 120,
                of: Some(120)
            }
        );
        let (to, _) = to.step(DownloadEvent::Progress(130, None));
        assert_eq!(
            to,
            Download::Running {
                done: 130,
                of: None
            }
        );
    }

    #[test]
    fn it_ends_saved_or_failed_and_dismiss_clears_either() {
        let (saved, _) = running().step(DownloadEvent::Saved("/tmp/a".into()));
        assert_eq!(saved, Download::Saved("/tmp/a".into()));
        assert_eq!(saved.step(DownloadEvent::Dismiss), (Download::Idle, None));
        let (bad, _) = running().step(DownloadEvent::Failed {
            retry: Retry::Now,
            why: "dropped".into(),
        });
        assert_eq!(bad, failed());
        assert_eq!(bad.step(DownloadEvent::Dismiss), (Download::Idle, None));
    }

    #[test]
    fn stray_events_do_nothing() {
        let cases = [
            (Download::Idle, DownloadEvent::Progress(1, None)),
            (Download::Idle, DownloadEvent::Saved("a".into())),
            (Download::Idle, DownloadEvent::Dismiss),
            (running(), DownloadEvent::Dismiss),
            (
                Download::Saved("a".into()),
                DownloadEvent::Progress(1, None),
            ),
        ];
        for (from, event) in cases {
            assert_eq!(
                from.step(event.clone()),
                (from.clone(), None),
                "{from:?} {event:?}"
            );
        }
    }
}
