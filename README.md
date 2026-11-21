# duxdm

A simple, fast **TUI display manager** for the Linux console — the login
flow of getty, plus a menu to pick your desktop.

```
login: username
password: ******
1. GNOME on Wayland  (wayland)
2. Sway               (wayland)
3. XFCE Session       (x11)
> pick one and press Enter
```

- **Wayland** sessions are executed directly on the current VT.
- **X11** sessions run through `xinit`.
- Authentication via **PAM** (the same `common-auth` stack as `login`/`getty`).
- One small Rust binary (~650 KB), no GUI toolkit, no config files, no bloat.

## Install (Debian/Ubuntu)

```sh
curl -fsSL https://webnotes.work/duxdm-repo/key.gpg | sudo tee /etc/apt/keyrings/duxdm.asc
echo 'deb [signed-by=/etc/apt/keyrings/duxdm.asc] https://webnotes.work/duxdm-repo stable main' | sudo tee /etc/apt/sources.list.d/duxdm.list
sudo apt update && sudo apt install duxdm
```

The package ships `/usr/bin/duxdm`, `/etc/pam.d/duxdm` and
`duxdm.service` (tty1, restarts on exit like getty) and enables it on
install. If you also have GDM/LightDM installed, duxdm takes over tty1 —
disable it again with `sudo systemctl disable duxdm.service`.

X11 sessions additionally need the `xinit` package; Wayland sessions need
nothing extra.

## Usage

1. Type your username at `login:`.
2. Type your password (not echoed).
3. Choose a session with ↑/↓ or 1–9, press Enter.
4. `^C` cancels a prompt; when you log out you land back at the login
   screen.

Sessions are discovered from `/usr/share/wayland-sessions` and
`/usr/share/xsessions` — anything that ships a `.desktop` file there shows
up in the menu automatically.

## How it works

- **Login screen:** canonical mode (line editing, like getty). Echo is
  turned off for the password. ISIG stays on so `^C` raises SIGINT; duxdm
  installs flag-setting signal handlers (no `SA_RESTART`), so the blocked
  read returns `EINTR` and the prompt cancels cleanly. SIGTERM/SIGHUP/
  SIGQUIT behave the same way — the tty is always restored on the way out.
- **Session menu:** raw mode, arrow keys, instant selection.
- **Session start:** after PAM authentication duxdm forks. The child
  restores the pre-duxdm termios, drops privileges
  (`initgroups` → `setgid` → `setuid`) and execs the session. Wayland runs
  directly; X11 is wrapped in `xinit` on the current VT. The parent waits
  for the session, closes the PAM session on logout and returns to the
  login screen.

## Building from source

```sh
# needs libpam0g-dev and libclang-dev (for the pam crate's bindgen)
cargo build --release
./scripts/build-deb.sh     # produces dist/duxdm_0.1.0-1_amd64.deb
```

## Testing

`tests/pty_e2e.py` drives the whole flow through a pseudo-terminal:
wrong-password rejection, password echo handling, session menu, session
exec, `^C` during password entry, SIGTERM, and terminal restoration.

```sh
sudo ./tests/setup.sh      # creates /etc/pam.d/duxdm, test user, fake sessions
sudo python3 tests/pty_e2e.py
```

## Publishing a new version

1. Bump `version` in `Cargo.toml` and `packaging/control`.
2. `./scripts/build-deb.sh && ./scripts/publish-apt.sh`
3. Rebuild + restart the webnotes site (the repo lives in its `public/`).

## License

MIT — see [LICENSE](LICENSE).
