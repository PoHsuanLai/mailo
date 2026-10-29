//! A client's threads end with the client: the last clone dropped leaves none of its `mailo-*`
//! threads behind. This binary holds one test, so every `mailo-*` thread of the process is a
//! client's.

use mail_client::{Client, ClientConfig};
use mail_store::testing::in_memory;
use std::time::{Duration, Instant};

/// The names of this process's live threads that start with `mailo-`.
fn mailo_threads() -> Vec<String> {
    std::fs::read_dir("/proc/self/task")
        .expect("the process lists its threads")
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .map(|name| name.trim().to_owned())
        .filter(|name| name.starts_with("mailo-"))
        .collect()
}

#[test]
fn a_dropped_client_leaves_no_thread_behind() {
    assert_eq!(mailo_threads(), Vec::<String>::new(), "before");
    let client = Client::open(in_memory().into(), ClientConfig::default());
    assert!(!mailo_threads().is_empty(), "an open client has threads");
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !mailo_threads().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(mailo_threads(), Vec::<String>::new(), "after the drop");
}
