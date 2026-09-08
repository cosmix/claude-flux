use std::collections::VecDeque;
use std::net::TcpStream;
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nix::errno::Errno;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::CloseFrame;
use tungstenite::{Error, Message, WebSocket};

use super::protocol::{
    clamp, parse_client_frame, ClientFrame, Mode, CLOSE_ENDED, CLOSE_SERVER_STOPPING,
};
use super::pty::PtyChild;

const READ_TIMEOUT: Duration = Duration::from_millis(5);
const WRITE_TIMEOUT: Duration = Duration::from_millis(250);
const POLL_TIMEOUT_MS: u16 = 50;
// Twin of the dashboard snapshot socket's cap in `web/ws.rs`.
const MAX_INBOUND_BYTES: usize = 64 * 1024;
const MAX_PENDING_INPUT: usize = 64 * 1024;

#[derive(Clone, Copy)]
struct Ready {
    master: PollFlags,
    tcp: PollFlags,
}

#[derive(Clone, Copy)]
enum Stop {
    Ended,
    ServerStopping,
    Peer,
}

/// Pump bytes between an accepted WebSocket and a PTY child until either side ends.
/// Consumes both. Returns when the socket is closed and the child is reaped.
pub(super) fn run(
    mut socket: tungstenite::WebSocket<TcpStream>,
    child: PtyChild,
    mode: Mode,
    running: &AtomicBool,
) {
    if !configure(&mut socket) {
        finish(socket, child, Stop::Peer);
        return;
    }
    let tcp_fd = socket.get_ref().as_raw_fd();
    let mut pending = VecDeque::new();
    let mut warned = false;
    let stop = loop {
        if !running.load(Ordering::SeqCst) {
            break Stop::ServerStopping;
        }
        let ready = match poll_ready(child.master(), tcp_fd, !pending.is_empty()) {
            Ok(ready) => ready,
            Err(Errno::EINTR) => continue,
            Err(_) => break Stop::Peer,
        };
        if let Some(stop) = pump_pty(&mut socket, &child, ready.master) {
            break stop;
        }
        if ready.tcp.contains(PollFlags::POLLIN) {
            if let Some(stop) = pump_socket(&mut socket, &child, mode, &mut pending, &mut warned) {
                break stop;
            }
        }
        if ready.master.contains(PollFlags::POLLOUT) && !drain_pending(&child, &mut pending) {
            break Stop::Peer;
        }
        if ready
            .tcp
            .intersects(PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL)
        {
            break Stop::Peer;
        }
    };
    finish(socket, child, stop);
}

fn configure(socket: &mut WebSocket<TcpStream>) -> bool {
    socket.set_config(|config| {
        config.max_message_size = Some(MAX_INBOUND_BYTES);
        config.max_frame_size = Some(MAX_INBOUND_BYTES);
    });
    let peer = socket.get_mut();
    peer.set_read_timeout(Some(READ_TIMEOUT)).is_ok()
        && peer.set_write_timeout(Some(WRITE_TIMEOUT)).is_ok()
}

fn poll_ready(master: BorrowedFd<'_>, tcp_fd: RawFd, write: bool) -> nix::Result<Ready> {
    let master_events = if write {
        PollFlags::POLLIN | PollFlags::POLLOUT
    } else {
        PollFlags::POLLIN
    };
    // Safety: `tcp_fd` remains owned by `socket` for the full bridge loop.
    let tcp = unsafe { BorrowedFd::borrow_raw(tcp_fd) };
    let mut fds = [
        PollFd::new(master, master_events),
        PollFd::new(tcp, PollFlags::POLLIN),
    ];
    poll(&mut fds, PollTimeout::from(POLL_TIMEOUT_MS))?;
    Ok(Ready {
        master: fds[0].revents().unwrap_or(PollFlags::empty()),
        tcp: fds[1].revents().unwrap_or(PollFlags::empty()),
    })
}

fn pump_pty(
    socket: &mut WebSocket<TcpStream>,
    child: &PtyChild,
    events: PollFlags,
) -> Option<Stop> {
    if events.contains(PollFlags::POLLIN) {
        let mut bytes = [0u8; 16 * 1024];
        match child.read(&mut bytes) {
            Ok(Some(0)) => return Some(Stop::Ended),
            Ok(Some(count))
                if socket
                    .send(Message::binary(bytes[..count].to_vec()))
                    .is_err() =>
            {
                return Some(Stop::Peer);
            }
            Ok(Some(_)) | Ok(None) => return None,
            Err(_) => return Some(Stop::Peer),
        }
    }
    if events.contains(PollFlags::POLLHUP) {
        Some(Stop::Ended)
    } else if events.intersects(PollFlags::POLLERR | PollFlags::POLLNVAL) {
        Some(Stop::Peer)
    } else {
        None
    }
}

fn pump_socket(
    socket: &mut WebSocket<TcpStream>,
    child: &PtyChild,
    mode: Mode,
    pending: &mut VecDeque<u8>,
    warned: &mut bool,
) -> Option<Stop> {
    loop {
        match socket.read() {
            Ok(Message::Close(_)) | Err(Error::ConnectionClosed | Error::AlreadyClosed) => {
                return Some(Stop::Peer);
            }
            Ok(message) => match parse_client_frame(&message) {
                Some(ClientFrame::Input(bytes)) if mode == Mode::Control => {
                    enqueue(pending, &bytes, warned);
                }
                Some(ClientFrame::Resize(size)) => {
                    let _ = child.resize(clamp(size));
                }
                Some(ClientFrame::Input(_)) | None => {}
            },
            Err(Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return None
            }
            Err(_) => return Some(Stop::Peer),
        }
    }
}

fn enqueue(pending: &mut VecDeque<u8>, bytes: &[u8], warned: &mut bool) {
    let overflowed = pending.len().saturating_add(bytes.len()) > MAX_PENDING_INPUT;
    if bytes.len() >= MAX_PENDING_INPUT {
        pending.clear();
        pending.extend(&bytes[bytes.len() - MAX_PENDING_INPUT..]);
    } else {
        let discard = pending
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(MAX_PENDING_INPUT);
        drop(pending.drain(..discard));
        pending.extend(bytes);
    }
    if overflowed && !*warned {
        tracing::debug!("browser terminal input queue overflowed; dropping oldest bytes");
        *warned = true;
    }
}

fn drain_pending(child: &PtyChild, pending: &mut VecDeque<u8>) -> bool {
    while !pending.is_empty() {
        let result = {
            let (front, back) = pending.as_slices();
            child.write_some(if front.is_empty() { back } else { front })
        };
        match result {
            Ok(0) => return true,
            Ok(count) => {
                drop(pending.drain(..count));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return true,
            Err(_) => return false,
        }
    }
    true
}

fn finish(mut socket: WebSocket<TcpStream>, child: PtyChild, stop: Stop) {
    let frame = match stop {
        Stop::Ended => Some(CloseFrame {
            code: CloseCode::from(CLOSE_ENDED),
            reason: "ended".into(),
        }),
        Stop::ServerStopping => Some(CloseFrame {
            code: CloseCode::from(CLOSE_SERVER_STOPPING),
            reason: "server stopping".into(),
        }),
        Stop::Peer => None,
    };
    if let Some(frame) = frame {
        let _ = socket.send(Message::Close(Some(frame)));
    }
    let _ = socket.close(None);
    let _ = socket.flush();
    child.shutdown();
}
