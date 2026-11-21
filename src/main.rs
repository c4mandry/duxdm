//! duxdm — a simple, fast TUI display manager for the Linux console.
//!
//! Usage is just like getty: type a username, then the password, then pick
//! a desktop environment / window manager. Wayland sessions are executed
//! directly on the current VT; X11 sessions are started through xinit.

mod auth;
mod launch;
mod sessions;
mod signals;
mod tui;

use std::process;

fn main() {
    // A display manager has to authenticate other users and change uid —
    // refuse to run unprivileged instead of failing mysteriously later.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("duxdm: must run as root (it logs in as other users).");
        process::exit(1);
    }

    signals::install();

    let state = match tui::terminal_setup() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("duxdm: terminal setup failed: {e}");
            process::exit(1);
        }
    };

    loop {
        // ^C (or SIGTERM from systemd): leave cleanly, terminal restored.
        if signals::interrupted() {
            break;
        }

        // A finished session may have left the tty in raw mode or with
        // echo off; reset to getty-like line editing before every prompt.
        tui::apply_login_mode(&state);

        let login = match tui::login_screen(&state) {
            Ok(l) => l,
            Err(tui::LoginOutcome::Cancelled) => break, // ^C / ^D / EOF
        };

        match auth::authenticate(&login.username, &login.password) {
            Err(e) => {
                println!("\nLogin incorrect ({e})");
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
            Ok(mut client) => match tui::session_menu(&sessions::discover()) {
                None => drop(client), // menu aborted: close the PAM session
                Some(entry) => {
                    launch::run_session(&entry, &login.username, &mut client, &state);
                    drop(client); // user logged out: close the PAM session
                }
            },
        }
    }

    tui::terminal_restore(&state);
}
