use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use crate::process::is_process_alive;

use super::protocol::WindowSize;
use super::pty::PtyChild;

fn spawn_shell(script: &str) -> PtyChild {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    PtyChild::spawn(&mut command, WindowSize { cols: 80, rows: 24 }).expect("spawn shell in PTY")
}

fn write_bytes(child: &PtyChild, bytes: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut written = 0;
    while written < bytes.len() && Instant::now() < deadline {
        match child.write_some(&bytes[written..]).expect("write PTY") {
            0 => thread::sleep(Duration::from_millis(10)),
            count => written += count,
        }
    }
    assert_eq!(written, bytes.len(), "PTY did not accept test input");
}

fn read_until(child: &PtyChild, needle: &[u8], timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut output = Vec::new();
    while Instant::now() < deadline {
        let mut bytes = [0u8; 4096];
        match child.read(&mut bytes).expect("read PTY") {
            Some(0) => break,
            Some(count) => output.extend_from_slice(&bytes[..count]),
            None => thread::sleep(Duration::from_millis(10)),
        }
        if output.windows(needle.len()).any(|part| part == needle) {
            break;
        }
    }
    output
}

#[test]
fn pty_child_echoes_input() {
    let child = spawn_shell("read line; printf 'got:%s\\n' \"$line\"");
    write_bytes(&child, b"hi\n");
    let output = read_until(&child, b"got:hi", Duration::from_secs(10));
    child.shutdown();
    assert!(
        output.windows(6).any(|part| part == b"got:hi"),
        "PTY output was {:?}",
        String::from_utf8_lossy(&output)
    );
}

#[test]
fn pty_child_resize_is_visible_to_stty() {
    let child = spawn_shell("read x; stty size");
    child
        .resize(WindowSize {
            cols: 120,
            rows: 40,
        })
        .expect("resize PTY");
    write_bytes(&child, b"\n");
    let output = read_until(&child, b"40 120", Duration::from_secs(10));
    child.shutdown();
    assert!(
        output.windows(6).any(|part| part == b"40 120"),
        "PTY output was {:?}",
        String::from_utf8_lossy(&output)
    );
}

#[test]
fn pty_child_shutdown_reaps_the_child() {
    let child = spawn_shell("sleep 30");
    let pid = child.pid();
    let started = Instant::now();
    child.shutdown();
    // A clean hangup finishes in milliseconds; `shutdown`'s kill fallback
    // takes 2 seconds. This bound must stay below that deadline, or the
    // assertion passes whether the hangup worked or the child was killed.
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(!is_process_alive(pid), "child {pid} remains alive");
}

#[test]
fn pty_child_resize_delivers_sigwinch() {
    let child = spawn_shell("trap 'stty size' WINCH; read x");
    thread::sleep(Duration::from_millis(100));
    child
        .resize(WindowSize {
            cols: 120,
            rows: 40,
        })
        .expect("resize PTY");
    let output = read_until(&child, b"40 120", Duration::from_secs(5));
    child.shutdown();
    assert!(
        output.windows(6).any(|part| part == b"40 120"),
        "PTY output was {:?}",
        String::from_utf8_lossy(&output)
    );
}

#[test]
fn pty_write_survives_a_child_that_stops_reading() {
    let child = spawn_shell("sleep 30");
    let payload = vec![b'x'; 1024 * 1024];
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saturated = false;
    while Instant::now() < deadline {
        match child.write_some(&payload) {
            Ok(0) => {
                saturated = true;
                break;
            }
            Ok(_) => {}
            Err(error) => panic!("write to non-reading PTY failed: {error}"),
        }
    }
    child.shutdown();
    assert!(saturated, "PTY write did not report backpressure");
}
