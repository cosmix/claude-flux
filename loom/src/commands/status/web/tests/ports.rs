//! Port-selection tests use real loopback listeners to exercise bind failures.

use std::io::ErrorKind;
use std::net::TcpListener;

use super::skip_without_loopback;
use crate::commands::status::web::{
    bind_first_available_loopback_port, bind_listener, DEFAULT_PORT,
};

fn occupy_port_with_successor() -> TcpListener {
    for _ in 0..8 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("occupy an ephemeral port");
        if listener.local_addr().expect("occupied address").port() < u16::MAX {
            return listener;
        }
    }
    panic!("could not obtain an ephemeral port with a successor");
}

#[test]
fn automatic_port_selection_advances_past_an_occupied_port() {
    if skip_without_loopback("automatic_port_selection_advances_past_an_occupied_port") {
        return;
    }
    let occupied = occupy_port_with_successor();
    let start_port = occupied.local_addr().expect("occupied address").port();

    let selected = bind_first_available_loopback_port(start_port)
        .expect("select a port after the occupied start");
    assert!(
        selected.local_addr().expect("selected address").port() > start_port,
        "automatic selection must advance after AddrInUse"
    );
    drop(selected);
    drop(occupied);
}

#[test]
fn automatic_port_selection_starts_at_the_default_port() {
    if skip_without_loopback("automatic_port_selection_starts_at_the_default_port") {
        return;
    }
    let probe = match TcpListener::bind(("127.0.0.1", DEFAULT_PORT)) {
        Ok(listener) => listener,
        Err(error) if error.kind() == ErrorKind::AddrInUse => {
            eprintln!("skipping: default dashboard port {DEFAULT_PORT} is already occupied");
            return;
        }
        Err(error) => panic!("probe default dashboard port {DEFAULT_PORT}: {error}"),
    };
    drop(probe);

    let selected = bind_listener(None).expect("bind an available default dashboard port");
    assert_eq!(
        selected.local_addr().expect("selected address").port(),
        DEFAULT_PORT,
        "automatic selection must begin at the documented default port"
    );
}

#[test]
fn explicit_zero_binds_an_ephemeral_port() {
    if skip_without_loopback("explicit_zero_binds_an_ephemeral_port") {
        return;
    }
    let listener = bind_listener(Some(0)).expect("bind explicit ephemeral port");
    assert_ne!(listener.local_addr().expect("listener address").port(), 0);
}

#[test]
fn explicit_port_does_not_fall_back_when_occupied() {
    if skip_without_loopback("explicit_port_does_not_fall_back_when_occupied") {
        return;
    }
    let occupied = TcpListener::bind("127.0.0.1:0").expect("occupy an ephemeral port");
    let port = occupied.local_addr().expect("occupied address").port();

    let error = bind_listener(Some(port)).expect_err("explicit occupied port must fail");
    assert!(
        error.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == ErrorKind::AddrInUse)
        }),
        "expected AddrInUse for explicit port {port}: {error:#}"
    );
}
