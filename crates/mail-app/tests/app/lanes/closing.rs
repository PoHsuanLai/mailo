//! Closing a window: the client it opened ends with it. Every lane `Window` asserts this when it
//! is dropped; this lane says so on its own.

use super::window::Window;

#[test]
fn a_closed_window_leaves_no_client_running() {
    let window = Window::open(|_| {});
    let clients = window.clients();
    assert_eq!(clients.alive(), 1, "an open window has its client");
    drop(window);
    assert_eq!(
        clients.alive(),
        0,
        "a dropped window took its client with it"
    );
}
