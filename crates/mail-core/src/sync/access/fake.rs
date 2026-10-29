//! A protocol with no server, for the tests of what is built on [`Access`]: it signs in to
//! nothing, fetches nothing, and waits as a script says.

use super::{Access, Answer, Pass, Wait};
use crate::sync::live::Lost;
use crate::sync::report::Done;
use crate::sync::run::Run;
use mail_runtime::{Cancel, RuntimeError, Woke};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

/// What one wait does.
pub(crate) enum Step {
    /// Wake with this after this long.
    After(Duration, Woke),
}

/// An [`Access`] whose waits follow a script. Once the script has run out a wait parks, as one on
/// a quiet server does, until it is cancelled.
pub(crate) struct Fake {
    steps: Mutex<Option<VecDeque<Step>>>,
}

impl Fake {
    pub fn waiting(steps: impl IntoIterator<Item = Step>) -> Self {
        Fake {
            steps: Mutex::new(Some(steps.into_iter().collect())),
        }
    }
}

impl Access for Fake {
    fn pass<'a>(&'a self, _run: &'a Run<'a>, _pass: Pass<'a>) -> Answer<'a, Done> {
        Box::pin(std::future::ready(Ok(Done::default())))
    }

    fn wait<'a>(&'a self, _run: &'a Run<'a>, _: &'a str) -> Answer<'a, Box<dyn Wait>, Lost> {
        // Waited on a second time, the fake has no more script: it waits to be cancelled.
        let steps = self.steps.lock().unwrap().take().unwrap_or_default();
        let script: Box<dyn Wait> = Box::new(Script { steps });
        Box::pin(std::future::ready(Ok(script)))
    }
}

/// The wait a [`Fake`] hands out.
struct Script {
    steps: VecDeque<Step>,
}

impl Wait for Script {
    fn wait<'a>(&'a mut self, cancel: &'a mut Cancel) -> Answer<'a, Woke, RuntimeError> {
        Box::pin(async move {
            if let Some(Step::After(after, woke)) = self.steps.pop_front() {
                tokio::select! {
                    () = tokio::time::sleep(after) => return Ok(woke),
                    _ = cancel.changed() => return Err(RuntimeError::Cancelled),
                }
            }
            // Either way the wait ends cancelled: a closed channel means its watch has gone.
            let _ = cancel.changed().await;
            Err(RuntimeError::Cancelled)
        })
    }
}
