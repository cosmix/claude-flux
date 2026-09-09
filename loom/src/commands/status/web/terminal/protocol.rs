use serde::Deserialize;
use tungstenite::Message;

/// Whether the browser may type into the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    View,
    Control,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WindowSize {
    pub cols: u16,
    pub rows: u16,
}

/// One message from the browser.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ClientFrame {
    Input(Vec<u8>),
    Resize(WindowSize),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResizeMessage {
    resize: ResizeBody,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResizeBody {
    cols: u16,
    rows: u16,
}

/// Binary frames are raw keystrokes; text frames are resize JSON.
pub(super) fn parse_client_frame(message: &Message) -> Option<ClientFrame> {
    match message {
        Message::Binary(bytes) => Some(ClientFrame::Input(bytes.to_vec())),
        Message::Text(text) => {
            let wire: ResizeMessage = serde_json::from_str(text.as_str()).ok()?;
            let size = WindowSize {
                cols: wire.resize.cols,
                rows: wire.resize.rows,
            };
            (clamp(size) == size).then_some(ClientFrame::Resize(size))
        }
        _ => None,
    }
}

/// Clamp to what a PTY can hold: 1..=500 columns, 1..=200 rows.
pub(super) fn clamp(size: WindowSize) -> WindowSize {
    WindowSize {
        cols: size.cols.clamp(1, 500),
        rows: size.rows.clamp(1, 200),
    }
}

/// The agent's tmux client exited normally.
pub(super) const CLOSE_ENDED: u16 = 1000;
pub(super) const CLOSE_SERVER_STOPPING: u16 = 1001;
/// The browser sent a frame past `max_message_size`, so the input stream is
/// no longer in sync. Named on the wire because an oversize paste is
/// otherwise indistinguishable from a dropped connection.
pub(super) const CLOSE_TOO_LARGE: u16 = 1009;
/// No stage with this id exists. Permanent; the page never retries.
pub(super) const CLOSE_UNKNOWN_STAGE: u16 = 4004;
/// The stage exists but can never be attached in this shape.
pub(super) const CLOSE_REFUSED: u16 = 4008;
/// The stage exists and may become attachable. The page retries.
pub(super) const CLOSE_NOT_YET: u16 = 4009;
