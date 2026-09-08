use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::pty::{openpty, Winsize};

use super::protocol::WindowSize;

/// A child process whose controlling terminal is a PTY we own.
pub(super) struct PtyChild {
    master: OwnedFd,
    child: Child,
}

impl PtyChild {
    /// Open a sized PTY and give its slave to the child as its controlling terminal.
    pub(super) fn spawn(command: &mut Command, size: WindowSize) -> std::io::Result<Self> {
        let winsize = winsize(size);
        let pair = openpty(Some(&winsize), None).map_err(io::Error::from)?;
        let flags = fcntl(&pair.master, FcntlArg::F_GETFL).map_err(io::Error::from)?;
        let flags = OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK;
        fcntl(&pair.master, FcntlArg::F_SETFL(flags)).map_err(io::Error::from)?;

        command.stdin(Stdio::from(pair.slave.try_clone()?));
        command.stdout(Stdio::from(pair.slave.try_clone()?));
        command.stderr(Stdio::from(pair.slave.try_clone()?));
        // Safety: only async-signal-safe session/ioctl operations run after fork.
        unsafe {
            command.pre_exec(|| {
                nix::unistd::setsid().map_err(io::Error::from)?;
                libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                Ok(())
            });
        }
        let child = command.spawn()?;
        drop(pair.slave);
        // `command` outlives this call in every caller (it drives the bridge
        // session for as long as the WebSocket is open), and `Command` keeps
        // the stdio fds it was given alive until it is dropped or reassigned.
        // Left alone, the three slave clones above would stay open for that
        // whole lifetime, so the master would never see the child's hangup
        // when it exits. Overwriting them here drops those clones now that
        // the child has its own copies from `spawn`.
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        Ok(Self {
            master: pair.master,
            child,
        })
    }

    pub(super) fn master(&self) -> BorrowedFd<'_> {
        self.master.as_fd()
    }

    /// Change the PTY window size, causing the child session to receive SIGWINCH.
    pub(super) fn resize(&self, size: WindowSize) -> io::Result<()> {
        let winsize = winsize(size);
        // Safety: the master fd is live and the pointer references a complete winsize.
        let result = unsafe {
            libc::ioctl(
                self.master.as_raw_fd(),
                libc::TIOCSWINSZ,
                &winsize as *const _,
            )
        };
        if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Read once without blocking; `None` means that no bytes are ready.
    pub(super) fn read(&self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        match nix::unistd::read(&self.master, buf) {
            Ok(count) => Ok(Some(count)),
            Err(Errno::EAGAIN) => Ok(None),
            Err(Errno::EIO) => Ok(Some(0)),
            Err(error) => Err(io::Error::from(error)),
        }
    }

    /// Write once without blocking; zero means that the PTY is not writable yet.
    pub(super) fn write_some(&self, bytes: &[u8]) -> io::Result<usize> {
        match nix::unistd::write(&self.master, bytes) {
            Ok(count) => Ok(count),
            Err(Errno::EAGAIN) => Ok(0),
            Err(error) => Err(io::Error::from(error)),
        }
    }

    #[cfg(test)]
    pub(super) fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Hang up the PTY, wait up to two seconds, then kill and reap the child.
    pub(super) fn shutdown(self) {
        let Self { master, mut child } = self;
        drop(master);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    thread::sleep(Duration::from_millis(50).min(remaining));
                }
                Ok(None) | Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn winsize(size: WindowSize) -> Winsize {
    Winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}
