//! A message's body: stored here, or still on the server.

use mail_domain::Retry;

/// Whether a message's body can be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// Stored. Terminal: a body that is here does not go back to the server.
    Here,
    /// Only the headers are stored.
    Missing,
    /// Being fetched.
    Fetching,
    /// The fetch failed, with what to do about it.
    Failed { retry: Retry, why: String },
}

/// What happened to the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyEvent {
    /// Someone wants to read it.
    Ask,
    /// It is stored now, however it got here.
    Arrived,
    Failed {
        retry: Retry,
        why: String,
    },
}

/// What the caller must do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyEffect {
    Fetch,
}

impl Body {
    /// What `event` does to the body.
    pub fn step(&self, event: BodyEvent) -> (Body, Option<BodyEffect>) {
        match (self, event) {
            (Body::Here, _) => (Body::Here, None),
            (Body::Missing | Body::Failed { .. }, BodyEvent::Ask) => {
                (Body::Fetching, Some(BodyEffect::Fetch))
            }
            (_, BodyEvent::Arrived) => (Body::Here, None),
            (Body::Fetching, BodyEvent::Failed { retry, why }) => {
                (Body::Failed { retry, why }, None)
            }
            _ => (self.clone(), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed() -> Body {
        Body::Failed {
            retry: Retry::Now,
            why: "dropped".into(),
        }
    }

    #[test]
    fn asking_fetches_what_is_missing_or_failed() {
        for from in [Body::Missing, failed()] {
            assert_eq!(
                from.step(BodyEvent::Ask),
                (Body::Fetching, Some(BodyEffect::Fetch))
            );
        }
    }

    #[test]
    fn asking_twice_is_one_fetch() {
        assert_eq!(Body::Fetching.step(BodyEvent::Ask), (Body::Fetching, None));
    }

    #[test]
    fn a_body_that_is_here_stays() {
        let events = [
            BodyEvent::Ask,
            BodyEvent::Arrived,
            BodyEvent::Failed {
                retry: Retry::Now,
                why: "x".into(),
            },
        ];
        for event in events {
            assert_eq!(
                Body::Here.step(event.clone()),
                (Body::Here, None),
                "{event:?}"
            );
        }
    }

    #[test]
    fn arriving_wins_from_any_other_state() {
        for from in [Body::Missing, Body::Fetching, failed()] {
            assert_eq!(from.step(BodyEvent::Arrived), (Body::Here, None));
        }
    }

    #[test]
    fn a_failed_fetch_keeps_what_to_do_about_it() {
        let event = BodyEvent::Failed {
            retry: Retry::NeedsReauth,
            why: "no".into(),
        };
        let (to, fx) = Body::Fetching.step(event);
        assert_eq!(
            (to, fx),
            (
                Body::Failed {
                    retry: Retry::NeedsReauth,
                    why: "no".into()
                },
                None
            )
        );
        let event = BodyEvent::Failed {
            retry: Retry::Now,
            why: "late".into(),
        };
        assert_eq!(Body::Missing.step(event), (Body::Missing, None));
    }
}
