//! Terminal handling and the two UI screens:
//!   login_screen()  — username + password prompts (canonical mode, getty-like)
//!   session_menu()  — arrow-key list of available DE/WM sessions (raw mode)

use std::io::{self, Write};
use std::os::fd::{AsRawFd, BorrowedFd};

use nix::sys::termios::{tcgetattr, tcsetattr, SetArg, Termios};
use nix::sys::termios::{ControlFlags, InputFlags, LocalFlags};

use crate::signals;

pub struct TerminalState {
    pub(crate) fd: i32,
    pub(crate) original: Termios,
}

pub enum LoginOutcome {
    Cancelled,
}

pub struct Login {
    pub username: String,
    pub password: String,
}

/// nix 0.32 termios functions take `impl AsFd` — wrap our raw fd.
fn as_fd(fd: i32) -> BorrowedFd<'static> {
    // SAFETY: the fd stays open for the whole duxdm process lifetime.
    unsafe { BorrowedFd::borrow_raw(fd) }
}

/// Save the terminal state and put the tty into getty-like canonical mode
/// (line editing with echo; ISIG stays on so ^C raises SIGINT, which our
/// flag-setting handler turns into a cancel).
pub fn terminal_setup() -> io::Result<TerminalState> {
    let fd = io::stdin().as_raw_fd();
    let original = tcgetattr(as_fd(fd)).map_err(io_err)?;

    let state = TerminalState { fd, original };
    apply_login_mode(&state);
    Ok(state)
}

/// Restore whatever the terminal looked like before duxdm started.
pub fn terminal_restore(state: &TerminalState) {
    let _ = tcsetattr(as_fd(state.fd), SetArg::TCSANOW, &state.original);
}

/// Canonical mode with echo — normal line editing like agetty. ISIG stays
/// on: ^C raises SIGINT, the handler sets the interrupt flag, the blocked
/// read returns EINTR and the prompt cancels. ICRNL is kept enabled: the
/// line discipline maps Enter (CR) to NL, which is what terminates a
/// canonical-mode line.
pub fn apply_login_mode(state: &TerminalState) {
    let mut t = state.original.clone();
    t.control_flags &= !(ControlFlags::CSIZE | ControlFlags::PARENB);
    t.control_flags |= ControlFlags::CS8;
    t.input_flags &= !(InputFlags::ISTRIP | InputFlags::INLCR | InputFlags::IGNCR | InputFlags::IXON);
    t.input_flags |= InputFlags::ICRNL;
    t.local_flags &= !(LocalFlags::ICANON | LocalFlags::ECHO | LocalFlags::ECHONL);
    t.local_flags |= LocalFlags::ICANON | LocalFlags::ECHO | LocalFlags::ISIG;
    let _ = tcsetattr(as_fd(state.fd), SetArg::TCSANOW, &t);
}

fn set_raw(state: &TerminalState) -> io::Result<()> {
    let mut t = state.original.clone();
    t.control_flags &= !(ControlFlags::CSIZE | ControlFlags::PARENB);
    t.control_flags |= ControlFlags::CS8;
    t.input_flags &= !(InputFlags::ISTRIP | InputFlags::INLCR | InputFlags::IGNCR | InputFlags::ICRNL | InputFlags::IXON);
    t.local_flags &= !(LocalFlags::ICANON | LocalFlags::ECHO | LocalFlags::ECHONL | LocalFlags::ISIG);
    tcsetattr(as_fd(state.fd), SetArg::TCSANOW, &t).map_err(io_err)
}

fn set_echo(state: &TerminalState, on: bool) -> io::Result<()> {
    let mut t = tcgetattr(as_fd(state.fd)).map_err(io_err)?;
    if on {
        t.local_flags |= LocalFlags::ECHO;
    } else {
        t.local_flags &= !LocalFlags::ECHO;
    }
    tcsetattr(as_fd(state.fd), SetArg::TCSANOW, &t).map_err(io_err)
}

/// Read one line in canonical mode. `echo` controls duxdm's own character
/// echo (the password prompt passes false; the tty's ECHO flag is off there
/// too). Returns None on EOF, ^C/^D or an interrupt signal.
fn read_line(echo: bool) -> io::Result<Option<String>> {
    let fd = io::stdin().as_raw_fd();
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    loop {
        if signals::interrupted() {
            return Ok(None); // ^C / SIGTERM / SIGHUP / SIGQUIT
        }
        match nix::unistd::read(as_fd(fd), &mut b) {
            Ok(0) => return Ok(None), // EOF: the tty went away
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue, // loop top checks the flag
            Err(e) => return Err(io_err(e)),
        }
        match b[0] {
            b'\n' | b'\r' => {
                println!();
                return Ok(Some(String::from_utf8_lossy(&buf).trim().to_string()));
            }
            0x03 | 0x04 => {
                // ^C / ^D cancels the prompt
                println!();
                return Ok(None);
            }
            0x7f | 0x08 => {
                // backspace
                if buf.pop().is_some() {
                    if echo {
                        print!("\x08 \x08");
                        io::stdout().flush()?;
                    }
                }
            }
            _ => {
                // Printable only — no control characters in a username/password.
                if (0x20..0x7f).contains(&b[0]) {
                    buf.push(b[0]);
                    if echo {
                        print!("{}", b[0] as char);
                        io::stdout().flush()?;
                    }
                }
            }
        }
    }
}

/// Username + password prompt, exactly like getty. Returns
/// Err(LoginOutcome::Cancelled) on ^C/^D/EOF.
pub fn login_screen(state: &TerminalState) -> Result<Login, LoginOutcome> {
    print!("\x1b[2J\x1b[H");
    print!("\x1b[1mduxdm\x1b[0m — log in\n\n");
    io::stdout().flush().ok();

    loop {
        print!("login: ");
        io::stdout().flush().ok();
        let Some(username) = read_line(true).ok().flatten() else {
            return Err(LoginOutcome::Cancelled);
        };
        if username.is_empty() {
            continue;
        }

        print!("password: ");
        io::stdout().flush().ok();
        // Hide the password while typing; restore echo right after.
        set_echo(state, false).ok();
        let Some(password) = read_line(false).ok().flatten() else {
            set_echo(state, true).ok();
            return Err(LoginOutcome::Cancelled);
        };
        set_echo(state, true).ok();
        if password.is_empty() {
            println!("\nLogin incorrect");
            continue;
        }

        return Ok(Login { username, password });
    }
}

/// Interactive DE/WM picker. Returns the chosen entry or None when aborted.
pub fn session_menu(entries: &[crate::sessions::SessionEntry]) -> Option<crate::sessions::SessionEntry> {
    if entries.is_empty() {
        println!();
        println!("No desktop sessions found.");
        println!("Install a DE/WM, or check /usr/share/wayland-sessions and");
        println!("/usr/share/xsessions for .desktop entries.");
        std::thread::sleep(std::time::Duration::from_secs(3));
        return None;
    }

    let fd = io::stdin().as_raw_fd();
    let original = tcgetattr(as_fd(fd)).ok()?;
    let state = TerminalState { fd, original: original.clone() };
    set_raw(&state).ok()?;
    let result = menu_loop(entries);
    // Restore the terminal regardless of how the menu ended.
    let _ = tcsetattr(as_fd(fd), SetArg::TCSANOW, &original);
    result
}

fn menu_loop(entries: &[crate::sessions::SessionEntry]) -> Option<crate::sessions::SessionEntry> {
    let mut sel = 0usize;
    let mut b = [0u8; 16];
    let fd = io::stdin().as_raw_fd();
    loop {
        draw_menu(entries, sel);
        let n = match nix::unistd::read(as_fd(fd), &mut b) {
            Ok(n) => n,
            Err(nix::errno::Errno::EINTR) => {
                if signals::interrupted() {
                    return None;
                }
                continue;
            }
            Err(_) => return None,
        };
        if n == 0 {
            return None;
        }
        match b[0] {
            b'q' | 0x03 | 0x04 => return None,
            0x1b if n >= 3 && b[1] == b'[' => match b[2] {
                b'A' => sel = sel.saturating_sub(1),
                b'B' => sel = (sel + 1).min(entries.len() - 1),
                _ => {}
            },
            b'\n' | b'\r' => return Some(entries[sel].clone()),
            c @ b'1'..=b'9' => {
                let idx = (c - b'1') as usize;
                if idx < entries.len() {
                    return Some(entries[idx].clone());
                }
            }
            _ => {}
        }
    }
}

fn draw_menu(entries: &[crate::sessions::SessionEntry], sel: usize) {
    print!("\x1b[2J\x1b[H");
    println!("\x1b[1mduxdm\x1b[0m — choose a session\n");
    for (i, e) in entries.iter().enumerate() {
        let kind = if e.wayland { "wayland" } else { "x11" };
        if i == sel {
            println!("\x1b[7m> {}. {}\x1b[0m  \x1b[2m({})\x1b[0m", i + 1, e.name, kind);
        } else {
            println!("  {}. {}  \x1b[2m({})\x1b[0m", i + 1, e.name, kind);
        }
    }
    println!("\n↑/↓ or 1-9 to select, Enter to start, q to abort.");
    io::stdout().flush().ok();
}

fn io_err(e: nix::errno::Errno) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}
