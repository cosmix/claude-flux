//! Stage-to-tmux target resolution for browser terminal attachments.

use std::path::Path;

use anyhow::Result;

use crate::commands::attach::{matches_for_stage, pick_newest};
use crate::commands::status::data::load_all_sessions;
use crate::fs::work_dir::WorkDir;
use crate::models::session::{Session, SessionBackendKind};
use crate::orchestrator::terminal::tmux::socket_name;
use crate::orchestrator::terminal::tmux::viewer::{
    endpoint_ready, is_plain_identifier, live_tmux_sessions, tmux_session_name,
};

use super::{Mode, CLOSE_NOT_YET, CLOSE_REFUSED, CLOSE_UNKNOWN_STAGE};

/// A validated tmux target for one browser terminal.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub socket: String,
    pub tmux_session: String,
}

/// Why a stage cannot be attached right now.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    UnknownStage,
    NativeBackend,
    Unrenderable,
    NoSession,
    NotReady,
}

impl Refusal {
    pub(super) fn close_code(&self) -> u16 {
        match self {
            Self::UnknownStage => CLOSE_UNKNOWN_STAGE,
            Self::NativeBackend | Self::Unrenderable => CLOSE_REFUSED,
            Self::NoSession | Self::NotReady => CLOSE_NOT_YET,
        }
    }

    pub(super) fn reason(&self) -> &'static str {
        match self {
            Self::UnknownStage => "no stage with that id",
            Self::NativeBackend => {
                "this stage's session runs in a native terminal window; terminals need loom run --backend tmux"
            }
            Self::Unrenderable => "this session's identifier cannot name a tmux session",
            Self::NoSession => "no live session for this stage yet",
            Self::NotReady => "still spawning, or just ended",
        }
    }
}

/// Resolve `stage_id` against the already-live tmux session population.
pub(super) fn resolve(
    stage_id: &str,
    known_stage: bool,
    all_sessions: &[Session],
    live: &[Session],
    ready: impl Fn(&Session, &str) -> bool,
) -> std::result::Result<Target, Refusal> {
    if !known_stage {
        return Err(Refusal::UnknownStage);
    }
    let matches = matches_for_stage(live, stage_id);
    let Some(session) = pick_newest(&matches) else {
        return Err(refusal_without_live_session(all_sessions, stage_id));
    };
    target_for(session, ready)
}

fn refusal_without_live_session(all_sessions: &[Session], stage_id: &str) -> Refusal {
    let session = all_sessions
        .iter()
        .filter(|session| {
            session.stage_id.as_deref() == Some(stage_id) && !session.status.is_terminal()
        })
        .max_by_key(|session| session.created_at);
    match session {
        Some(session) if session.backend != SessionBackendKind::Tmux => Refusal::NativeBackend,
        _ => Refusal::NoSession,
    }
}

fn target_for(
    session: &Session,
    ready: impl Fn(&Session, &str) -> bool,
) -> std::result::Result<Target, Refusal> {
    let Some(tmux_session) = tmux_session_name(session) else {
        return Err(Refusal::Unrenderable);
    };
    let socket = socket_name(session);
    if !is_plain_identifier(&socket) || !is_plain_identifier(&tmux_session) {
        return Err(Refusal::Unrenderable);
    }
    if !ready(session, &tmux_session) {
        return Err(Refusal::NotReady);
    }
    Ok(Target {
        socket,
        tmux_session,
    })
}

/// Resolve `stage_id` against the work dir's session and tmux state on disk.
pub(super) fn resolve_target(
    stage_id: &str,
    base: &Path,
) -> Result<std::result::Result<Target, Refusal>> {
    let work_dir = WorkDir::new(base)?;
    let known = crate::verify::transitions::load_stage(stage_id, work_dir.root()).is_ok();
    let all = load_all_sessions(&work_dir)?;
    let live = live_tmux_sessions(work_dir.root())?;
    Ok(resolve(stage_id, known, &all, &live, endpoint_ready))
}

/// Arguments for the tmux attach client, excluding the `tmux` executable.
pub(super) fn attach_args(target: &Target, mode: Mode) -> Vec<String> {
    let mut args = vec![
        "-L".to_owned(),
        target.socket.clone(),
        "-T".to_owned(),
        "256,RGB".to_owned(),
        "attach-session".to_owned(),
    ];
    if matches!(mode, Mode::View) {
        args.extend(["-f".to_owned(), "read-only".to_owned()]);
    }
    args.extend(["-t".to_owned(), target.tmux_session.clone()]);
    args
}
