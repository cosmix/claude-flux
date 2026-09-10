//! Static shell-text constants shared by `build_wrapper_script`, split out to
//! keep `wrapper.rs` under its line cap.

/// Rebuilds the child environment from a minimal host allowlist rather than
/// inheriting it, so ambient credentials and token-shaped variables never
/// reach a stage session. Fully static — no interpolation.
///
/// TERMINFO/TERMINFO_DIRS travel alongside TERM, together with HOME (already
/// forwarded, which covers `~/.terminfo`) — the standard ncurses resolution
/// inputs. TERM only names the terminal; these say where its capability
/// database lives, and forwarding the name without the database forwards
/// half a contract: any terminal whose terminfo entry is not bundled into
/// the system database (kitty is the observed instance) leaves the stage
/// agent's inherited TERM with nowhere to resolve.
/// SCCACHE_DIR/SCCACHE_CACHE_SIZE forward an operator's own sccache cache config.
/// RUSTC_WRAPPER is deliberately absent here — `sccache_env` decides it instead,
/// using the session's sandbox verdict this allowlist has no access to.
pub(super) fn env_allowlist() -> &'static str {
    r#"# Reconstruct the stage environment from a minimal host allowlist. In
# particular, ambient credentials and token-shaped variables are not inherited.
_loom_env=(
    "HOME=${HOME:-}"
    "PATH=${PATH:-/usr/bin:/bin}"
)
for _loom_name in LANG LC_ALL LC_CTYPE TERM TERMINFO TERMINFO_DIRS COLORTERM \
    TERM_PROGRAM SHELL DISPLAY \
    WAYLAND_DISPLAY XAUTHORITY DBUS_SESSION_BUS_ADDRESS XDG_RUNTIME_DIR \
    TMUX_TMPDIR TMUX TMUX_PANE TMPDIR \
    SCCACHE_DIR SCCACHE_CACHE_SIZE; do
    _loom_value="${!_loom_name}"
    if [ -n "$_loom_value" ]; then
        _loom_env+=("$_loom_name=$_loom_value")
    fi
done
"#
}
