//! Control/view mode round trips, detach, and shutdown behavior for the
//! browser terminal.

use std::sync::atomic::Ordering;
use std::time::Duration;

use serial_test::serial;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::Message;

use crate::tmux_backend::wait_until;

use super::{close_code, fixture, read_binary_for, terminal_socket};

#[test]
#[serial]
fn control_mode_round_trips_a_command() {
    let Some(mut fixture) = fixture("control_mode_round_trips_a_command") else {
        return;
    };
    let mut socket = terminal_socket(&fixture, "control");
    socket
        .send(Message::text(r#"{"resize":{"cols":80,"rows":24}}"#))
        .expect("send resize");
    socket
        .send(Message::binary(b"printf 'hi-%s-web\\n' from\r".to_vec()))
        .expect("send command");
    let output = read_binary_for(&mut socket, Duration::from_secs(10));
    assert!(
        output
            .windows(b"hi-from-web".len())
            .any(|part| part == b"hi-from-web"),
        "the tmux shell did not run the command: {}",
        String::from_utf8_lossy(&output)
    );
    socket.close(None).expect("close control socket");
    drop(socket);
    fixture.stop_server(Duration::from_secs(5));
}

#[test]
#[serial]
fn view_mode_receives_the_screen_but_never_types() {
    let Some(mut fixture) = fixture("view_mode_receives_the_screen_but_never_types") else {
        return;
    };
    let mut socket = terminal_socket(&fixture, "view");
    socket
        .send(Message::binary(b"printf never-typed\r".to_vec()))
        .expect("send view-mode input");
    let output = read_binary_for(&mut socket, Duration::from_secs(2));
    assert!(
        !output.is_empty(),
        "view mode should receive the tmux screen"
    );
    assert!(
        !output
            .windows(b"never-typed".len())
            .any(|part| part == b"never-typed"),
        "view mode forwarded browser input"
    );
    socket.close(None).expect("close view socket");
    drop(socket);
    fixture.stop_server(Duration::from_secs(5));
}

#[test]
#[serial]
fn detaching_reaps_the_tmux_client() {
    let Some(mut fixture) = fixture("detaching_reaps_the_tmux_client") else {
        return;
    };
    let mut socket = terminal_socket(&fixture, "control");
    assert!(
        !read_binary_for(&mut socket, Duration::from_secs(1)).is_empty(),
        "control attachment should receive a tmux screen"
    );
    socket.close(None).expect("close control socket");
    drop(socket);
    assert!(
        wait_until(|| fixture.clients_empty(), Duration::from_secs(3)),
        "tmux client remained after the browser detached"
    );
    fixture.stop_server(Duration::from_secs(5));
}

#[test]
#[serial]
fn server_stop_closes_with_1001_and_reaps() {
    let Some(mut fixture) = fixture("server_stop_closes_with_1001_and_reaps") else {
        return;
    };
    let mut socket = terminal_socket(&fixture, "control");
    assert!(
        !read_binary_for(&mut socket, Duration::from_secs(1)).is_empty(),
        "control attachment should receive a tmux screen"
    );
    fixture.running.store(false, Ordering::SeqCst);
    assert_eq!(
        close_code(&mut socket, Duration::from_secs(3)),
        Some(CloseCode::Away),
        "server stop should close with 1001"
    );
    drop(socket);
    fixture.stop_server(Duration::from_secs(5));
    assert!(
        fixture.clients_empty(),
        "tmux client remained after server stop"
    );
}
