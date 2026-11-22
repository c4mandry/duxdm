//! Launch the chosen session: drop privileges, set up the environment,
//! then exec. Wayland sessions run directly on the current VT. X11 sessions
//! get a root-owned X server on the current VT (display-manager style) while
//! the session itself runs as the user.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

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

/// The user's runtime directory. Wayland compositors create their socket
/// inside it; the user's D-Bus session bus lives there too. pam_systemd
/// normally creates it during open_session — ensure it regardless.
fn ensure_runtime_dir(uid: u32, gid: u32) -> String {
    let dir = format!("/run/user/{uid}");
    match std::fs::create_dir(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => eprintln!("duxdm: could not create {dir}: {e}"),
    }
    if std::fs::metadata(&dir).map(|m| m.is_dir()).unwrap_or(false) {
        let _ = nix::unistd::chown(
            Path::new(&dir),
            Some(Uid::from_raw(uid)),
            Some(Gid::from_raw(gid)),
        );
        let _ = std::fs::set_permissions(&dir, PermissionsExt::from_mode(0o700));
    }
    dir
}

/// Environment handed to the session. Mirrors what getty + pam_systemd
/// provide for a normal login (plus the Wayland/X11-specific variables).
fn session_env(user: &User, username: &str, entry: &SessionEntry, vt: &str) -> Vec<(String, String)> {
    let home = user.dir.to_string_lossy().into_owned();
    let shell = user.shell.to_string_lossy().into_owned();
    let runtime = ensure_runtime_dir(user.uid.as_raw(), user.gid.as_raw());
    let bus_path = format!("{runtime}/bus");

    let mut env = vec![
        ("HOME".into(), home),
        ("USER".into(), username.into()),
        ("LOGNAME".into(), username.into()),
        ("SHELL".into(), shell),
        ("PATH".into(), DEFAULT_PATH.into()),
        ("XDG_RUNTIME_DIR".into(), runtime),
        ("XDG_SESSION_CLASS".into(), "user".into()),
        (
            "XDG_SESSION_TYPE".into(),
            if entry.wayland { "wayland" } else { "x11" }.into(),
        ),
        ("XDG_SESSION_DESKTOP".into(), entry.name.clone()),
        ("XDG_SEAT".into(), "seat0".into()),
        ("XDG_VTNR".into(), vt.into()),
    ];
    if !entry.desktop_names.is_empty() {
        env.push(("XDG_CURRENT_DESKTOP".into(), entry.desktop_names.clone()));
    }
    // The user bus appears shortly after pam_systemd starts user@<uid>.service.
    if Path::new(&bus_path).exists() {
        env.push(("DBUS_SESSION_BUS_ADDRESS".into(), format!("unix:path={bus_path}")));
    }
    if let Ok(term) = std::env::var("TERM") {
        env.push(("TERM".into(), term));
    }
    env.push((
        "LANG".into(),
        std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into()),
    ));
    env
}

/// Drop to the user: supplementary groups, gid, then uid.
fn drop_privileges(user: &User) {
    let cname = std::ffi::CString::new(user.name.clone()).expect("user name contains no NUL");
    let _ = nix::unistd::initgroups(&cname, Gid::from_raw(user.gid.as_raw()));
    let _ = nix::unistd::setgid(Gid::from_raw(user.gid.as_raw()));
    let _ = nix::unistd::setuid(Uid::from_raw(user.uid.as_raw()));
}

/// waitpid that retries on EINTR (a signal must not make us abandon a child).
fn wait_retry(pid: i32, status: &mut i32) -> i32 {
    loop {
        let r = unsafe { libc::waitpid(pid, status, 0) };
        if r != -1 {
            return r;
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return r;
        }
    }
}

/// Poll until a path appears (bounded).
fn wait_for_path(path: &str, timeout: Duration) {
    let start = Instant::now();
    while !Path::new(path).exists() && start.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 16 random bytes as a lowercase hex string (X11 auth cookie).
fn random_cookie_hex() -> String {
    let mut buf = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut buf))
        .is_err()
    {
        // Fallback: time + pid mixed in; good enough to keep the session
        // isolated even without /dev/urandom.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seed = (now as u64) ^ ((std::process::id() as u64) << 32);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (seed >> (i * 8)) as u8 ^ 0xA5;
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write an Xauthority file (standard Xau format): family 256 (FamilyLocal),
/// hostname as address, the display number, the protocol name
/// "MIT-MAGIC-COOKIE-1", and the 16-byte cookie. The display name itself is
/// NOT stored — xauth identifies the display by family+address+number.
fn write_xauthority(path: &str, cookie_hex: &str, display: &str) -> std::io::Result<()> {
    let mut host = [0u8; 256];
    let host = if unsafe { libc::gethostname(host.as_mut_ptr() as *mut libc::c_char, 256) } == 0 {
        let end = host.iter().position(|&b| b == 0).unwrap_or(host.len());
        host[..end].to_vec()
    } else {
        b"localhost".to_vec()
    };

    let number = display.trim_start_matches(':'); // e.g. "0"
    let proto = b"MIT-MAGIC-COOKIE-1";

    let mut buf = Vec::with_capacity(48 + host.len() + number.len());
    buf.extend_from_slice(&256u16.to_be_bytes()); // FamilyLocal
    buf.extend_from_slice(&(host.len() as u16).to_be_bytes());
    buf.extend_from_slice(&host);
    buf.extend_from_slice(&(number.len() as u16).to_be_bytes());
    buf.extend_from_slice(number.as_bytes());
    buf.extend_from_slice(&(proto.len() as u16).to_be_bytes());
    buf.extend_from_slice(proto);
    buf.extend_from_slice(&16u16.to_be_bytes());
    for i in (0..32).step_by(2) {
        buf.push(u8::from_str_radix(&cookie_hex[i..i + 2], 16).unwrap_or(0));
    }
    std::fs::write(path, buf)
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

    let vt = current_vt();
    let env = session_env(&user, username, entry, &vt);

    if entry.wayland {
        match unsafe { libc::fork() } {
            -1 => eprintln!("duxdm: fork failed"),
            0 => {
                // Child: give the tty back its pre-duxdm settings (the
                // compositor expects a sane terminal), become the user,
                // exec the session. Keep the controlling terminal (no
                // setsid) so the compositor owns this VT.
                let _ = tcsetattr(
                    // SAFETY: the fd stays open for the whole process lifetime.
                    unsafe { std::os::fd::BorrowedFd::borrow_raw(state.fd) },
                    SetArg::TCSANOW,
                    &state.original,
                );
                drop_privileges(&user);
                let mut c = Command::new("/bin/sh");
                c.arg("-c").arg(&entry.exec);
                c.envs(env);
                let err = c.exec();
                eprintln!("duxdm: failed to start session: {err}");
                std::process::exit(1);
            }
            pid => {
                let mut status = 0;
                wait_retry(pid, &mut status);
            }
        }
        return;
    }

    run_x11_session(entry, &user, &env, &vt, state);
}

/// X11: duxdm starts the X server itself, as root, on the current VT —
/// the server can take the VT without depending on logind/Xorg.wrap, and
/// the session still runs as the user (display-manager architecture).
fn run_x11_session(
    entry: &SessionEntry,
    user: &User,
    env: &[(String, String)],
    vt: &str,
    state: &TerminalState,
) {
    // DUXDM_XSERVER overrides the server binary (test hook).
    let xorg = std::env::var_os("DUXDM_XSERVER")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| find_command(DEFAULT_PATH, "Xorg"));
    let Some(xorg) = xorg else {
        eprintln!("duxdm: Xorg not found — install xserver-xorg-core for X11 sessions");
        return;
    };

    // Pick a display whose socket is not already in use.
    let display = if Path::new("/tmp/.X11-unix/X0").exists() { 1 } else { 0 };
    let display_name = format!(":{display}");

    let cookie = random_cookie_hex();
    let server_auth = format!("/tmp/duxdm-server-auth-{}", std::process::id());
    let user_auth = format!("/tmp/duxdm-user-auth-{}", std::process::id());
    if write_xauthority(&server_auth, &cookie, &display_name).is_err()
        || write_xauthority(&user_auth, &cookie, &display_name).is_err()
    {
        eprintln!("duxdm: could not write Xauthority files");
        return;
    }
    // The user's copy must be readable by the user session.
    let _ = nix::unistd::chown(
        Path::new(&user_auth),
        Some(Uid::from_raw(user.uid.as_raw())),
        Some(Gid::from_raw(user.gid.as_raw())),
    );

    let server_pid = unsafe { libc::fork() };
    match server_pid {
        -1 => {
            eprintln!("duxdm: fork failed (X server)");
            let _ = std::fs::remove_file(&server_auth);
            let _ = std::fs::remove_file(&user_auth);
        }
        0 => {
            // Root child: exec the X server on the current VT.
            let _ = tcsetattr(
                // SAFETY: the fd stays open for the whole process lifetime.
                unsafe { std::os::fd::BorrowedFd::borrow_raw(state.fd) },
                SetArg::TCSANOW,
                &state.original,
            );
            let mut c = Command::new(&xorg);
            c.arg(&display_name)
                .arg(format!("vt{vt}"))
                .arg("-keeptty")
                .arg("-nolisten")
                .arg("tcp")
                .arg("-auth")
                .arg(&server_auth)
                .arg("-background")
                .arg("none");
            let err = c.exec();
            eprintln!("duxdm: X server failed to start: {err}");
            std::process::exit(1);
        }
        server => {
            let session_pid = unsafe { libc::fork() };
            match session_pid {
                -1 => {
                    eprintln!("duxdm: fork failed (session)");
                    unsafe { libc::kill(server, libc::SIGTERM) };
                    let mut s = 0;
                    wait_retry(server, &mut s);
                    let _ = std::fs::remove_file(&server_auth);
                    let _ = std::fs::remove_file(&user_auth);
                }
                0 => {
                    // User child: wait for the server socket, then run the
                    // session against it.
                    let _ = tcsetattr(
                        // SAFETY: as above.
                        unsafe { std::os::fd::BorrowedFd::borrow_raw(state.fd) },
                        SetArg::TCSANOW,
                        &state.original,
                    );
                    drop_privileges(user);
                    let socket = format!("/tmp/.X11-unix/X{display}");
                    wait_for_path(&socket, Duration::from_secs(5));
                    let mut c = Command::new("/bin/sh");
                    c.arg("-c").arg(&entry.exec);
                    c.envs(env.iter().cloned())
                        .env("DISPLAY", &display_name)
                        .env("XAUTHORITY", &user_auth);
                    let err = c.exec();
                    eprintln!("duxdm: failed to start session: {err}");
                    std::process::exit(1);
                }
                session => {
                    // Parent: wait for the user's session to end, then stop
                    // the X server and clean up the auth files.
                    let mut status = 0;
                    wait_retry(session, &mut status);
                    unsafe { libc::kill(server, libc::SIGTERM) };
                    let mut s = 0;
                    wait_retry(server, &mut s);
                    let _ = std::fs::remove_file(&server_auth);
                    let _ = std::fs::remove_file(&user_auth);
                }
            }
        }
    }
}
