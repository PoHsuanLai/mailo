//! The pusher over a private bus, against a stand-in for the router's `Index` that keeps epochs as
//! the router does (docket-router's `index_step`): a push in the epoch the last `Reset` started is
//! taken, any other is refused as `Malformed`. Nothing here reaches the person's session bus or
//! mail: what is pushed is made in the test.

use super::*;
use crate::intents::tests::Bus;
use chrono::TimeZone;
use mail_core::findable::{Findable, Kind};
use std::sync::{Arc, Mutex, PoisonError};

/// Pushes are quick here, so a test waits on them, not on the clock.
const SOON: Duration = Duration::from_millis(10);

/// How long a test waits for the pusher before it fails.
const PATIENCE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Heard {
    Reset(u64),
    Push {
        epoch: u64,
        upserts: Vec<String>,
        removals: Vec<String>,
    },
    Refused(u64),
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.quire.Intents1.Error")]
enum Refusal {
    #[zbus(error)]
    ZBus(zbus::Error),
    Malformed(String),
}

#[derive(Default)]
struct Router {
    epoch: Option<u64>,
    heard: Vec<Heard>,
}

#[derive(Clone, Default)]
struct Index(Arc<Mutex<Router>>);

impl Index {
    fn router(&self) -> std::sync::MutexGuard<'_, Router> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn heard(&self) -> Vec<Heard> {
        self.router().heard.clone()
    }

    /// What the router does when it restarts, or marks the app stale.
    fn forget(&self) {
        self.router().epoch = None;
    }

    /// Wait until `done` holds of what was heard, and answer that.
    fn until(&self, what: &str, done: impl Fn(&[Heard]) -> bool) -> Vec<Heard> {
        let start = Instant::now();
        loop {
            let heard = self.heard();
            if done(&heard) {
                return heard;
            }
            assert!(start.elapsed() < PATIENCE, "{what}: heard {heard:?}");
            std::thread::sleep(SOON);
        }
    }
}

#[zbus::interface(name = "org.quire.Intents1.Index")]
impl Index {
    fn reset(&self, epoch: u64) {
        let mut router = self.router();
        router.epoch = Some(epoch);
        router.heard.push(Heard::Reset(epoch));
    }

    fn push(&self, batch: String) -> Result<(), Refusal> {
        let batch: serde_json::Value = serde_json::from_str(&batch).expect("a batch");
        let epoch = batch["epoch"].as_u64().expect("an epoch");
        let mut router = self.router();
        if router.epoch != Some(epoch) {
            router.heard.push(Heard::Refused(epoch));
            return Err(Refusal::Malformed("another epoch".to_owned()));
        }
        let keys = |field: &str| -> Vec<String> {
            batch[field]
                .as_array()
                .expect(field)
                .iter()
                .map(|item| {
                    item.get("key")
                        .unwrap_or(item)
                        .as_str()
                        .expect("a key")
                        .to_owned()
                })
                .collect()
        };
        let (upserts, removals) = (keys("upserts"), keys("removals"));
        router.heard.push(Heard::Push {
            epoch,
            upserts,
            removals,
        });
        Ok(())
    }
}

fn connect(bus: &Bus) -> Connection {
    zbus::blocking::connection::Builder::address(bus.address.as_str())
        .expect("an address")
        .build()
        .expect("a connection")
}

/// The router, on `bus`, under its name.
fn router_on(bus: &Bus, index: &Index) -> Connection {
    zbus::blocking::connection::Builder::address(bus.address.as_str())
        .expect("an address")
        .serve_at(PATH, index.clone())
        .expect("served")
        .name(ROUTER)
        .expect("the router's name")
        .build()
        .expect("the router")
}

fn contact(n: usize) -> Findable {
    Findable {
        kind: Kind::Contact,
        key: format!("p{n}@example.test"),
        title: format!("Person {n}"),
        subtitle: format!("p{n}@example.test"),
        keywords: Vec::new(),
        updated: chrono::Utc
            .timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("a time"),
    }
}

/// What the store reads as, which a test changes.
#[derive(Clone, Default)]
struct Mail(Arc<Mutex<Findables>>);

impl Mail {
    fn of(n: usize) -> Mail {
        let mail = Mail::default();
        mail.change(|all| {
            for found in (0..n).map(contact) {
                all.insert(found.id(), found);
            }
        });
        mail
    }

    fn change(&self, edit: impl FnOnce(&mut Findables)) {
        edit(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner));
    }

    fn source(&self) -> impl FnMut() -> Option<Findables> + Send + 'static {
        let mail = self.clone();
        move || {
            Some(
                mail.0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone(),
            )
        }
    }
}

fn pushed(heard: &[Heard]) -> usize {
    heard
        .iter()
        .filter_map(|h| match h {
            Heard::Push { upserts, .. } => Some(upserts.len()),
            _ => None,
        })
        .sum()
}

#[test]
fn at_the_start_a_reset_then_everything_in_batches() {
    let bus = Bus::start();
    let index = Index::default();
    let _router = router_on(&bus, &index);
    let _changed = push_on(connect(&bus), Mail::of(1_203).source(), SOON).expect("pushing");
    let heard = index.until("everything", |heard| pushed(heard) == 1_203);
    let Some(Heard::Reset(epoch)) = heard.first().cloned() else {
        panic!("no reset first: {heard:?}");
    };
    let sizes: Vec<(u64, usize, usize)> = heard[1..]
        .iter()
        .map(|h| match h {
            Heard::Push {
                epoch,
                upserts,
                removals,
            } => (*epoch, upserts.len(), removals.len()),
            other => panic!("not a push: {other:?}"),
        })
        .collect();
    assert_eq!(sizes, [(epoch, 500, 0), (epoch, 500, 0), (epoch, 203, 0)]);
}

#[test]
fn a_change_pushes_only_what_changed() {
    let bus = Bus::start();
    let index = Index::default();
    let _router = router_on(&bus, &index);
    let mail = Mail::of(3);
    let changed = push_on(connect(&bus), mail.source(), SOON).expect("pushing");
    index.until("everything", |heard| pushed(heard) == 3);
    mail.change(|all| {
        all.remove(&contact(0).id());
        let mut renamed = contact(1);
        renamed.title = "Person One".to_owned();
        all.insert(renamed.id(), renamed);
        all.insert(contact(7).id(), contact(7));
    });
    changed.said();
    let heard = index.until("the change", |heard| heard.len() == 3);
    let Heard::Reset(epoch) = heard[0] else {
        panic!("no reset first: {heard:?}");
    };
    assert_eq!(
        heard[2],
        Heard::Push {
            epoch,
            upserts: vec!["p1@example.test".to_owned(), "p7@example.test".to_owned()],
            removals: vec!["p0@example.test".to_owned()],
        }
    );
    // Nothing moved, so a write that changed nothing pushes nothing.
    changed.said();
    std::thread::sleep(SOON * 20);
    assert_eq!(index.heard().len(), 3);
}

#[test]
fn a_push_refused_as_malformed_resets_and_pushes_everything_again() {
    let bus = Bus::start();
    let index = Index::default();
    let _router = router_on(&bus, &index);
    let mail = Mail::of(2);
    let changed = push_on(connect(&bus), mail.source(), SOON).expect("pushing");
    index.until("everything", |heard| pushed(heard) == 2);
    index.forget();
    mail.change(|all| {
        all.insert(contact(5).id(), contact(5));
    });
    changed.said();
    let heard = index.until("everything again", |heard| pushed(heard) == 5);
    let [
        Heard::Reset(first),
        _,
        Heard::Refused(refused),
        Heard::Reset(second),
        again,
    ] = heard.as_slice()
    else {
        panic!("not a refusal and a reset: {heard:?}");
    };
    assert_eq!(refused, first);
    assert_ne!(second, first);
    let Heard::Push {
        epoch,
        upserts,
        removals,
    } = again
    else {
        panic!("not a push: {again:?}");
    };
    assert_eq!((epoch, upserts.len(), removals.len()), (second, 3, 0));
}

#[test]
fn with_no_router_nothing_breaks_and_one_that_comes_gets_everything() {
    let bus = Bus::start();
    let reads = Arc::new(Mutex::new(0_usize));
    let mail = Mail::of(4);
    let mut read = mail.source();
    let counted = reads.clone();
    let reader = move || {
        *counted.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        read()
    };
    let changed = push_on(connect(&bus), reader, SOON).expect("pushing with no router");
    changed.said();
    std::thread::sleep(SOON * 20);
    assert_eq!(
        *reads.lock().unwrap_or_else(PoisonError::into_inner),
        0,
        "the store is read for nobody"
    );
    let index = Index::default();
    let _router = router_on(&bus, &index);
    let heard = index.until("everything", |heard| pushed(heard) == 4);
    assert!(matches!(heard[0], Heard::Reset(_)), "{heard:?}");
}
