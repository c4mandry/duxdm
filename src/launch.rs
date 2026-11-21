//! Launch the chosen session: drop privileges, set up the environment,
//! then exec. Wayland sessions run directly on the current VT; X11 sessions
//! go through xinit (which starts the X server and hands the session the
//! DISPLAY).

use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use nix::sys::termios::{tcsetattr, SetArg};
use nix::unistd::{Gid, Uid, User};
use pam::{Client, PasswordConv};

use crate::sessions::SessionEntry;
use crate::tui::TerminalState;

/// Standard PATH used for the user's session.
const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// The VT the console is currently on (e.g. "7" for tty7).
fn current_vt() -> String {
    std::fs::read_to_string("/sys/class/tty/tty0/active")
        .map(|s| s.trim().trim_start_matches("tty").to_string())
        .unwrap_or_else(|_| "1".to_string())
}

/// Find `name` as an executable somewhere on `path`.
fn find_command(path: &str, name: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .find(|d| nix::unistd::access(&d.join(name), nix::unistd::AccessFlags::X_OK).is_ok())
        .map(|d| d.join(name))
}

/// Fork; the child becomes the user and execs the session. The parent
/// (duxdm) waits for the child to exit and then returns, so main() can
/// close the PAM session and show the login screen again.
pub fn run_session(
    entry: &SessionEntry,
    username: &str,
    _client: &mut Client<'static, PasswordConv>,
    state: &TerminalState,
) {
    let user = match User::from_name(username) {
        Ok(Some(u)) => u,
        _ => {
            eprintln!("duxdm: unknown user {username}");
            return;
        }
    };

    // Build the environment BEFORE forking; execve replaces it in the child.
    let home = user.dir.to_string_lossy().into_owned();
    let shell = user.shell.to_string_lossy().into_owned();
    let mut env: Vec<(String, String)> = Vec::new();
    env.push(("HOME".into(), home));
    env.push(("USER".into(), username.into()));
    env.push(("LOGNAME".into(), username.into()));
    env.push(("SHELL".into(), shell));
    env.push(("PATH".into(), DEFAULT_PATH.into()));
    env.push(("XDG_SESSION_CLASS".into(), "user".into()));
    env.push((
        "XDG_SESSION_TYPE".into(),
        if entry.wayland { "wayland" } else { "x11" }.into(),
    ));
    env.push(("XDG_SESSION_DESKTOP".into(), entry.name.clone()));
    if !entry.desktop_names.is_empty() {
        env.push(("XDG_CURRENT_DESKTOP".into(), entry.desktop_names.clone()));
    }
    if let Ok(term) = std::env::var("TERM") {
        env.push(("TERM".into(), term));
    }
    env.push((
        "LANG".into(),
        std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into()),
    ));

    match unsafe { libc::fork() } {
        -1 => eprintln!("duxdm: fork failed"),
        0 => {
            // Child: give the tty back its pre-duxdm settings (the session
            // expects a sane terminal), become the user, then exec the
            // session. Keep the controlling terminal (no setsid) so the
            // DE/WM owns this VT.
            let _ = tcsetattr(
                // SAFETY: the fd stays open for the whole duxdm process lifetime.
                unsafe { std::os::fd::BorrowedFd::borrow_raw(state.fd) },
                SetArg::TCSANOW,
                &state.original,
            );

            let cname = std::ffi::CString::new(user.name.clone())
                .expect("user name contains no NUL");
            let _ = nix::unistd::initgroups(&cname, Gid::from_raw(user.gid.as_raw()));
            let _ = nix::unistd::setgid(Gid::from_raw(user.gid.as_raw()));
            let _ = nix::unistd::setuid(Uid::from_raw(user.uid.as_raw()));

            let err = exec_session(entry, env);
            eprintln!("duxdm: failed to start session: {err}");
            std::process::exit(1);
        }
        pid => {
            // Parent: wait for the user's session to end. Retry on EINTR —
            // a signal must not make us abandon the child while the PAM
            // session is still open.
            let mut status = 0;
            loop {
                let r = unsafe { libc::waitpid(pid, &mut status, 0) };
                if r != -1 {
                    break;
                }
                if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                    break;
                }
            }
        }
    }
}

/// exec the session command. Wayland runs directly; X11 is wrapped in
/// xinit so an X server starts on the current VT with a matching auth
/// cookie. Never returns on success.
fn exec_session(entry: &SessionEntry, env: Vec<(String, String)>) -> std::io::Error {
    if entry.wayland {
        return exec_shell_command(&entry.exec, env);
    }

    // X11 needs xinit to own the X server lifecycle.
    let Some(xinit) = find_command(DEFAULT_PATH, "xinit") else {
        return std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "xinit not found — install the xinit package for X11 sessions",
        );
    };
    let xserver = find_command(DEFAULT_PATH, "X").unwrap_or_else(|| PathBuf::from("/usr/bin/X"));

    // Write a tiny xinitrc and let xinit run the X server + session.
    let rc = crate::sessions::xinitrc_path();
    if std::fs::write(&rc, format!("exec {}\n", entry.exec)).is_ok() {
        let _ = std::fs::set_permissions(
            &rc,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        );
    }
    let vt = current_vt();
    let mut cmd = Command::new(xinit);
    cmd.arg(&rc)
        .arg("--")
        .arg(xserver)
        .arg(":0")
        .arg(format!("vt{vt}"))
        .arg("-keeptty")
        .arg("-nolisten")
        .arg("tcp");
    cmd.envs(env);
    cmd.exec()
}

/// Run a command through `sh -c` (session Exec lines are shell fragments
/// that may include arguments, env assignments, etc).
fn exec_shell_command(cmd: &str, env: Vec<(String, String)>) -> std::io::Error {
    let mut c = Command::new("/bin/sh");
    c.arg("-c").arg(cmd);
    c.envs(env);
    c.exec()
}
