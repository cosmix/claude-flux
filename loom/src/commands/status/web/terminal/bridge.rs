use std::collections::VecDeque;
use std::net::TcpStream;
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use tungstenite::error::CapacityError;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::CloseFrame;
use tungstenite::{Error, Message, WebSocket};

use super::protocol::{
    clamp, parse_client_frame, ClientFrame, Mode, CLOSE_ENDED, CLOSE_REFUSED,
    CLOSE_SERVER_STOPPING, CLOSE_TOO_LARGE,
};
use super::pty::PtyChild;
use crate::commands::status::web::limits::MAX_INBOUND_BYTES;

const READ_TIMEOUT: Duration = Duration::from_millis(5);
const WRITE_TIMEOUT: Duration = Duration::from_millis(250);
const POLL_TIMEOUT_MS: u16 = 50;
const MAX_PENDING_INPUT: usize = 64 * 1024;
/// How long the input queue may sit at `MAX_PENDING_INPUT` before the bridge
/// gives up on the child.
///
/// A gated bridge holds a terminal slot, a PTY and a tmux client, and neither
/// `poll` nor a socket read can tell whether the browser is still there: the
/// mask is narrowed, a FIN raises no POLLHUP, and while gated the unread
/// backlog is returned ahead of any EOF behind it. A wedged child never sends
/// a FIN in the first place. So the hold is bounded by elapsed time rather
/// than by a readiness flag, which also keeps one mechanism across Linux and
/// macOS. A live tmux client drains 64 KiB in milliseconds, so surviving this
/// only needs the PTY to accept roughly 2 KiB a second.
pub(super) const GATE_STALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Messages taken from the browser in one poll iteration. The
/// `MAX_PENDING_INPUT` gate already bounds a keystroke flood, but resize
/// frames and view-mode input never reach `pending`, so on their own they
/// would let a client hold the iteration open for as long as it keeps the
/// socket readable, starving PTY output and the `running` re-check.
const MAX_INBOUND_MESSAGES: usize = 64;
/// Ceiling on tungstenite's outbound buffer. A write that timed out is
/// retried rather than fatal (see `outbound_stop`), so this is what ends the
/// bridge when a client never resumes reading at all.
const MAX_WRITE_BUFFER: usize = 2 * 1024 * 1024;

struct Ready {
    master: PollFlags,
    tcp: PollFlags,
}

enum Stop {
    Ended,
    ServerStopping,
    Peer,
    TooLarge,
    Stalled,
}

/// Pump bytes between an accepted WebSocket and a PTY child until either side ends.
/// Consumes both. Returns when the socket is closed and the child is reaped.
pub(super) fn run(
    socket: tungstenite::WebSocket<TcpStream>,
    child: PtyChild,
    mode: Mode,
    running: &AtomicBool,
) {
    run_with(socket, child, mode, running, GATE_STALL_TIMEOUT);
}

/// `run` with the gate's give-up deadline supplied rather than taken from
/// `GATE_STALL_TIMEOUT`. Exists for the tests: covering that path otherwise
/// costs 30 s of wall clock per assertion.
pub(super) fn run_with(
    mut socket: tungstenite::WebSocket<TcpStream>,
    child: PtyChild,
    mode: Mode,
    running: &AtomicBool,
    stall_timeout: Duration,
) {
    if !configure(&mut socket) {
        finish(socket, child, Stop::Peer);
        return;
    }
    let tcp_fd = socket.get_ref().as_raw_fd();
    let mut pending = VecDeque::new();
    let mut buffered = false;
    let mut gated_since: Option<Instant> = None;
    let stop = loop {
        if !running.load(Ordering::SeqCst) {
            break Stop::ServerStopping;
        }
        if gate_stalled(&mut gated_since, pending.len(), stall_timeout) {
            break Stop::Stalled;
        }
        let ready = match poll_ready(child.master(), tcp_fd, pending.len()) {
            Ok(ready) => ready,
            Err(Errno::EINTR) => continue,
            Err(_) => break Stop::Peer,
        };
        if let Some(stop) = pump_pty(&mut socket, &child, ready.master) {
            break stop;
        }
        // `buffered` stands in for readiness `poll` cannot report: whole
        // messages left in the codec's own read buffer by a gated pass are
        // invisible at the fd, so waiting for POLLIN would strand them.
        if (ready.tcp.contains(PollFlags::POLLIN) || buffered) && pending.len() < MAX_PENDING_INPUT
        {
            if let Some(stop) = pump_socket(&mut socket, &child, mode, &mut pending, &mut buffered)
            {
                break stop;
            }
        }
        if ready.master.contains(PollFlags::POLLOUT) && !drain_pending(&child, &mut pending) {
            break Stop::Peer;
        }
        if socket_gone(ready.tcp) {
            break Stop::Peer;
        }
    };
    finish(socket, child, stop);
}

/// Track how long the input queue has sat at its cap, and say when that has
/// gone on too long.
///
/// Gating is normal and self-clearing while the child consumes, so the clock
/// starts on the first pass that finds the queue full and is thrown away the
/// moment it drains; only a gate that never reopens means nobody is draining
/// this terminal.
fn gate_stalled(since: &mut Option<Instant>, pending: usize, timeout: Duration) -> bool {
    if pending < MAX_PENDING_INPUT {
        *since = None;
        return false;
    }
    since.get_or_insert_with(Instant::now).elapsed() >= timeout
}

/// Whether `poll` reported the socket as gone rather than merely not ready.
/// These three arrive whether or not they were asked for, which is what lets
/// the gated event mask narrow to nothing without losing them.
fn socket_gone(events: PollFlags) -> bool {
    events.intersects(PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL)
}

fn configure(socket: &mut WebSocket<TcpStream>) -> bool {
    socket.set_config(|config| {
        config.max_message_size = Some(MAX_INBOUND_BYTES);
        config.max_frame_size = Some(MAX_INBOUND_BYTES);
        config.max_write_buffer_size = MAX_WRITE_BUFFER;
    });
    let peer = socket.get_mut();
    peer.set_read_timeout(Some(READ_TIMEOUT)).is_ok()
        && peer.set_write_timeout(Some(WRITE_TIMEOUT)).is_ok()
}

fn poll_ready(master: BorrowedFd<'_>, tcp_fd: RawFd, pending: usize) -> nix::Result<Ready> {
    let master_events = if pending > 0 {
        PollFlags::POLLIN | PollFlags::POLLOUT
    } else {
        PollFlags::POLLIN
    };
    // Backpressure: with the queue full, stop asking whether the socket is
    // readable, so the browser stalls against TCP's own receive window instead
    // of us buffering without bound. The gate belongs on the event mask, not
    // on the read call alone: skipping the read while still asking for POLLIN
    // makes `poll` return immediately every iteration and burns a core until
    // the client moves. What the gate costs is peer-close detection: a closing
    // tab sends a FIN, which raises POLLIN and, on Linux, POLLRDHUP - never
    // the POLLHUP that only a reset or a full shutdown produces. So the hangup
    // check in `run` cannot see a half-closed browser here. Reading the socket
    // cannot see one either while gated, since the unread backlog is the
    // backpressure and a peek returns that backlog rather than the EOF behind
    // it; `GATE_STALL_TIMEOUT` bounds the hold instead.
    let tcp_events = if pending < MAX_PENDING_INPUT {
        PollFlags::POLLIN
    } else {
        PollFlags::empty()
    };
    // Safety: `tcp_fd` remains owned by `socket` for the full bridge loop.
    let tcp = unsafe { BorrowedFd::borrow_raw(tcp_fd) };
    let mut fds = [
        PollFd::new(master, master_events),
        PollFd::new(tcp, tcp_events),
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
    // Finish a frame the client's receive window left half-written. Costs
    // nothing while the out-buffer is empty, and is the only thing that
    // resumes a stalled write once the PTY itself has gone quiet. A stall ends
    // the pass instead of falling through to the read: `send` below is a write
    // plus a flush, so carrying on would park on the same drain twice in one
    // iteration - unbounded, since `WRITE_TIMEOUT` is per `write(2)` and a
    // client taking a trickle never trips it - and would keep pulling 16 KiB a
    // pass off the master into the out-buffer, discarding tmux's own
    // backpressure to buffer toward `MAX_WRITE_BUFFER` here instead.
    if let Err(error) = socket.flush() {
        return outbound_stop(Err(error));
    }
    if events.contains(PollFlags::POLLIN) {
        let mut bytes = [0u8; 16 * 1024];
        match child.read(&mut bytes) {
            Ok(Some(0)) => return Some(Stop::Ended),
            Ok(Some(count)) => {
                return outbound_stop(socket.send(Message::binary(bytes[..count].to_vec())))
            }
            Ok(None) => return None,
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
    buffered: &mut bool,
) -> Option<Stop> {
    // Assume the codec still holds messages until a read says otherwise. Only
    // the `WouldBlock` arm below proves the socket and the codec's buffer are
    // both empty; every other way out of this pass is a gate, and the caller
    // has to come back for the rest without waiting on `poll`.
    *buffered = true;
    for _ in 0..MAX_INBOUND_MESSAGES {
        // Re-checked per message, not only before the call: one invocation
        // otherwise drains a receive buffer that already holds several
        // messages, so the queue peaks at that buffered total instead of at
        // the cap. With the check here the peak is one message past the cap.
        if pending.len() >= MAX_PENDING_INPUT {
            return None;
        }
        match socket.read() {
            Ok(Message::Close(_)) | Err(Error::ConnectionClosed | Error::AlreadyClosed) => {
                return Some(Stop::Peer);
            }
            Ok(message) => match parse_client_frame(&message) {
                Some(ClientFrame::Input(bytes)) if mode == Mode::Control => {
                    pending.extend(bytes);
                }
                Some(ClientFrame::Resize(size)) => {
                    let _ = child.resize(clamp(size));
                }
                Some(ClientFrame::Input(_)) | None => {}
            },
            // A paste past `MAX_INBOUND_BYTES`. Named on the wire rather than
            // folded into the bare close a dead peer gets, which the browser
            // cannot tell from a dropped connection.
            Err(Error::Capacity(CapacityError::MessageTooLong { .. })) => {
                return Some(Stop::TooLarge);
            }
            Err(Error::Io(error)) if stalled(&error) => {
                *buffered = false;
                return None;
            }
            Err(_) => return Some(Stop::Peer),
        }
    }
    None
}

/// Decide whether an outbound write ends the bridge.
///
/// A client that has stopped reading is not a dead one: `WRITE_TIMEOUT` expiry
/// leaves the frame in tungstenite's out-buffer for a later flush to resume,
/// so nothing is lost by retrying on the next iteration. Every other error
/// stays fatal, `Error::WriteBufferFull` above all - it returns immediately
/// instead of parking, so tolerating it would spin a core, and it is the
/// bound (`MAX_WRITE_BUFFER`) that terminates a stall that never clears.
fn outbound_stop(result: tungstenite::Result<()>) -> Option<Stop> {
    match result {
        Ok(()) => None,
        Err(Error::Io(error)) if stalled(&error) => None,
        Err(_) => Some(Stop::Peer),
    }
}

/// A transfer that has not happened yet, as opposed to one that failed.
fn stalled(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
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
        Stop::TooLarge => Some(CloseFrame {
            code: CloseCode::from(CLOSE_TOO_LARGE),
            reason: "message too large".into(),
        }),
        // 4008 rather than a bare close: the page already reports that code as
        // a refusal carrying the server's reason, where `Stop::Peer`'s absent
        // frame reads as tmux having exited normally.
        Stop::Stalled => Some(CloseFrame {
            code: CloseCode::from(CLOSE_REFUSED),
            reason: "input stalled".into(),
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
