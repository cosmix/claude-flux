//! Browser terminals: a PTY-hosted `tmux attach-session` client per WebSocket.
mod bridge;
mod protocol;
mod pty;
mod resolve;
#[cfg(test)]
mod tests_bridge;
#[cfg(test)]
mod tests_pty;
#[cfg(test)]
mod tests_route;
mod token;
mod upgrade;
use protocol::{Mode, WindowSize, CLOSE_NOT_YET, CLOSE_REFUSED, CLOSE_UNKNOWN_STAGE};
pub(crate) use upgrade::handle_upgrade;
