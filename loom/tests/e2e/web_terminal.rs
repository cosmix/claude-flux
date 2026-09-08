//! Fixtures backing the browser-terminal end-to-end tests, against a real
//! tmux server.
//!
//! ## Modules
//!
//! - `tests` - control/view mode round trips, detach, and shutdown behavior

use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use loom::commands::status::web::{serve, ServeOptions};
use loom::fs::work_dir::WorkDir;
use loom::models::session::{Session, SessionBackendKind, SessionStatus, SessionType};
use loom::models::stage::Stage;
use loom::orchestrator::terminal::tmux::{socket_name, socket_path_for};
use loom::verify::transitions::create_stage;
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::{Error, Message, WebSocket};
use wait_timeout::ChildExt;

use crate::helpers::create_session_file;
use crate::tmux_backend::{skip_unless_tmux_can_bind, TmuxServerGuard, TmuxTmpDirGuard};

mod tests;

const STAGE_ID: &str = "web-term-stage";
const TOKEN: &str = "e2e-token";
const TMUX_TIMEOUT: Duration = Duration::from_secs(3);

struct TmuxOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

struct Fixture {
    _temp: tempfile::TempDir,
    _tmux_tmpdir: TmuxTmpDirGuard,
    _tmux_server: TmuxServerGuard,
    socket: String,
    port: u16,
    running: Arc<AtomicBool>,
    done: Option<Receiver<anyhow::Result<()>>>,
    thread: Option<JoinHandle<()>>,
}

impl Fixture {
    fn clients_empty(&self) -> bool {
        let output = tmux(&self.socket, &["list-clients"]);
        assert!(
            output.success,
            "list tmux clients failed: {}",
            output.stderr
        );
        output.stdout.trim().is_empty()
    }

    fn stop_server(&mut self, timeout: Duration) {
        self.running.store(false, Ordering::SeqCst);
        let result = self
            .done
            .take()
            .expect("server completion receiver")
            .recv_timeout(timeout)
            .expect("server thread should finish before timeout");
        result.expect("web server should stop cleanly");
        self.thread
            .take()
            .expect("server thread")
            .join()
            .expect("web server thread should not panic");
    }

    fn stop_after_panic(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(done) = self.done.take() {
            let _ = done.recv_timeout(Duration::from_secs(5));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop_after_panic();
    }
}

fn tmux(socket: &str, args: &[&str]) -> TmuxOutput {
    let mut child = Command::new("tmux")
        .args(["-L", socket])
        .args(args)
        .env_remove("TMUX")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start tmux command");
    let status = match child
        .wait_timeout(TMUX_TIMEOUT)
        .expect("wait for tmux command")
    {
        Some(status) => status,
        None => {
            let _ = child.kill();
            child.wait().expect("reap timed out tmux command")
        }
    };
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("tmux stdout")
        .read_to_string(&mut stdout)
        .expect("read tmux stdout");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("tmux stderr")
        .read_to_string(&mut stderr)
        .expect("read tmux stderr");
    TmuxOutput {
        success: status.success(),
        stdout,
        stderr,
    }
}

fn skip_or_panic(test_name: &str, reason: &str) -> bool {
    if std::env::var("LOOM_E2E_REQUIRE_TMUX").as_deref() == Ok("1") {
        panic!("{test_name}: {reason} (LOOM_E2E_REQUIRE_TMUX=1 demands a real run)");
    }
    eprintln!("SKIP {test_name}: {reason}");
    true
}

fn start_tmux(session: &Session) -> (String, TmuxServerGuard) {
    let socket = socket_name(session);
    let output = tmux(
        &socket,
        &[
            "new-session",
            "-d",
            "-s",
            &session.tracking_key,
            "-x",
            "80",
            "-y",
            "24",
            "sh",
        ],
    );
    assert!(
        output.success,
        "start tmux server failed: {}",
        output.stderr
    );
    let guard = TmuxServerGuard {
        socket_path: socket_path_for(&socket),
    };
    (socket, guard)
}

fn prepare_workspace() -> (tempfile::TempDir, PathBuf, String, TmuxServerGuard) {
    let temp = tempfile::tempdir().expect("create temporary workspace");
    let base = temp.path().to_path_buf();
    let work_dir = WorkDir::new(&base).expect("build work dir");
    work_dir.initialize().expect("initialize work dir");

    let mut stage = Stage::new("Web terminal".to_owned(), None);
    stage.id = STAGE_ID.to_owned();
    create_stage(&stage, work_dir.root()).expect("create stage record");

    let mut session = Session::new();
    session.stage_id = Some(STAGE_ID.to_owned());
    session.tracking_key = Session::derive_tracking_key(STAGE_ID, SessionType::Stage);
    session.backend = SessionBackendKind::Tmux;
    session.status = SessionStatus::Running;
    session.pid = Some(std::process::id());
    create_session_file(&base, &session).expect("create session record");

    let pids = work_dir.root().join("pids");
    std::fs::create_dir_all(&pids).expect("create PID directory");
    let pid_key = format!("{}-{}", session.tracking_key, session.id);
    std::fs::write(
        pids.join(format!("{pid_key}.pid")),
        format!("{}\n", std::process::id()),
    )
    .expect("write PID identity");
    let (socket, tmux_server) = start_tmux(&session);
    (temp, base, socket, tmux_server)
}

fn fixture(test_name: &str) -> Option<Fixture> {
    let tmux_tmpdir = TmuxTmpDirGuard::new();
    if skip_unless_tmux_can_bind(tmux_tmpdir.dir(), test_name) {
        return None;
    }
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(_) => {
            skip_or_panic(test_name, "cannot bind loopback");
            return None;
        }
    };
    if which::which("tmux").is_err() {
        skip_or_panic(test_name, "tmux is not installed");
        return None;
    }
    let port = listener.local_addr().expect("server address").port();
    let (temp, base, socket, tmux_server) = prepare_workspace();
    let running = Arc::new(AtomicBool::new(true));
    let serve_running = running.clone();
    let (sender, done) = mpsc::sync_channel(1);
    let thread = thread::spawn(move || {
        let _ = sender.send(serve(
            listener,
            base,
            serve_running,
            ServeOptions {
                terminal_token: Some(TOKEN.to_owned()),
            },
        ));
    });
    Some(Fixture {
        _temp: temp,
        _tmux_tmpdir: tmux_tmpdir,
        _tmux_server: tmux_server,
        socket,
        port,
        running,
        done: Some(done),
        thread: Some(thread),
    })
}

fn terminal_socket(fixture: &Fixture, mode: &str) -> WebSocket<TcpStream> {
    let url = format!(
        "ws://127.0.0.1:{}/ws/terminal/{STAGE_ID}/{mode}",
        fixture.port
    );
    let mut request = url
        .as_str()
        .into_client_request()
        .expect("terminal request");
    request.headers_mut().insert(
        "Origin",
        format!("http://127.0.0.1:{}", fixture.port)
            .parse()
            .expect("origin header"),
    );
    request.headers_mut().insert(
        "Cookie",
        format!("loom_dashboard_{}={TOKEN}", fixture.port)
            .parse()
            .expect("cookie header"),
    );
    let stream = TcpStream::connect(("127.0.0.1", fixture.port)).expect("connect web server");
    let (mut socket, _) = tungstenite::client(request, stream).expect("terminal handshake");
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(250)))
        .expect("set WebSocket read timeout");
    socket
}

fn read_binary_for(socket: &mut WebSocket<TcpStream>, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut bytes = Vec::new();
    while Instant::now() < deadline {
        match socket.read() {
            Ok(Message::Binary(frame)) => bytes.extend_from_slice(&frame),
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) => {}
            Err(error) => panic!("read terminal frame: {error}"),
        }
    }
    bytes
}

fn close_code(socket: &mut WebSocket<TcpStream>, timeout: Duration) -> Option<CloseCode> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match socket.read() {
            Ok(Message::Close(Some(frame))) => return Some(frame.code),
            Ok(Message::Close(None)) => return None,
            Ok(_) => {}
            Err(Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) => {}
            Err(error) => panic!("read terminal close frame: {error}"),
        }
    }
    None
}
