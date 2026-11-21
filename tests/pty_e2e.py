#!/usr/bin/env python3
"""End-to-end duxdm test driven through a pseudo-terminal.

Run as root:
    ./tests/setup.sh          # once, with sudo
    sudo ./tests/pty_e2e.py

Exercises the full getty-like flow:
  1. wrong password  -> "Login incorrect" -> re-prompt
  2. echo off while the password is typed, echo restored afterwards
  3. session menu shows wayland + x11 entries
  4. picking a session execs it as the user, then returns to login
  5. ^C at the login prompt cancels cleanly (exit 0, tty restored)
"""

import fcntl
import os
import pty
import select
import struct
import sys
import termios
import time

BIN = os.environ.get(
    "DUXDM_BIN",
    os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
        "target",
        "release",
        "duxdm",
    ),
)
USER = "duxdmtest"
PW = "duxdmtest123"

FAILURES = []


def check(name, cond, extra=""):
    tag = "ok" if cond else "FAIL"
    print(f"[{tag}] {name}" + (f" — {extra}" if extra and not cond else ""))
    if not cond:
        FAILURES.append(name)


def expect(fd, needle, timeout=12.0, label=""):
    buf = b""
    deadline = time.time() + timeout
    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.2)
        if r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                chunk = b""
            if not chunk:
                break
            buf += chunk
            if needle.encode() in buf:
                return buf
    raise AssertionError(
        f"did not see {needle!r} ({label}) within {timeout}s; got: {buf!r}"
    )


def send(fd, data):
    os.write(fd, data)


def echo_state(fd):
    return bool(termios.tcgetattr(fd)[3] & termios.ECHO)


def wait_echo_off(fd, timeout=3.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if not echo_state(fd):
            return True
        time.sleep(0.05)
    return False


def spawn():
    pid, master = pty.fork()
    if pid == 0:
        os.environ["DUXDM_WAYLAND_SESSIONS"] = "/tmp/duxdm-test/wayland-sessions"
        os.environ["DUXDM_XSESSIONS"] = "/tmp/duxdm-test/xsessions"
        os.environ["TERM"] = "linux"
        os.execv(BIN, [BIN])
    return pid, master


def main():
    pid, master = spawn()

    pending = b""

    def expect_buffered(fd, needle, timeout=12.0, label=""):
        nonlocal pending
        deadline = time.time() + timeout
        while needle.encode() not in pending:
            if time.time() > deadline:
                raise AssertionError(
                    f"did not see {needle!r} ({label}) within {timeout}s; got: {pending!r}"
                )
            r, _, _ = select.select([fd], [], [], 0.2)
            if r:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:
                    chunk = b""
                if not chunk:
                    break
                pending += chunk
        if needle.encode() not in pending:
            raise AssertionError(
                f"did not see {needle!r} ({label}); got: {pending!r}"
            )
        return pending

    try:
        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        orig_echo = echo_state(master)

        # 1. first prompt
        expect_buffered(master, "login:", label="first login prompt")

        # 2. wrong password
        send(master, USER.encode() + b"\r")
        expect_buffered(master, "password:", label="password prompt (bad pw)")
        check("password echo off while typing", wait_echo_off(master))
        send(master, b"WRONG\r")
        expect_buffered(master, "Login incorrect", label="bad password rejected")
        expect_buffered(master, "login:", label="re-prompt after bad password")
        check("echo restored after bad login", echo_state(master))

        # 3. correct password
        send(master, USER.encode() + b"\r")
        expect_buffered(master, "password:", label="password prompt (good pw)")
        check("password echo off (good pw)", wait_echo_off(master))
        send(master, PW.encode() + b"\r")
        expect_buffered(master, "choose a session", label="session menu")
        expect_buffered(master, "TestSession", label="wayland entry listed")
        expect_buffered(master, "Broken X11", label="x11 entry listed")

        # 4. start the fake wayland session (entry 1)
        send(master, b"1\r")
        expect_buffered(master, "DUXDM-SESSION-RAN", label="session executed")
        pending = b""  # drop stale output; wait for the FRESH login prompt
        expect_buffered(master, "login:", label="back at login after session")
        check("echo restored after session", echo_state(master))

        # 5. ^C while entering the password cancels cleanly — the classic
        #    trap: the tty must not be left with echo off afterwards.
        send(master, USER.encode() + b"\r")
        expect_buffered(master, "password:", label="password prompt (^C test)")
        check("password echo off (^C test)", wait_echo_off(master))
        send(master, b"\x03")
        _, status = os.waitpid(pid, 0)
        check("clean exit on ^C during password (code 0)",
              os.waitstatus_to_exitcode(status) == 0,
              extra=f"exit={os.waitstatus_to_exitcode(status)}")
        check("tty restored after ^C (echo back on)", echo_state(master))

        # 6. SIGTERM (as from systemd) also exits cleanly with the tty restored.
        pid2, master2 = spawn()
        pending = b""  # drop stale output; wait for duxdm #2 to be ready
        expect_buffered(master2, "login:", label="login prompt (sigterm test)")
        os.kill(pid2, 15)
        _, status = os.waitpid(pid2, 0)
        check("clean exit on SIGTERM", os.waitstatus_to_exitcode(status) == 0,
              extra=f"exit={os.waitstatus_to_exitcode(status)}")
        check("tty restored after SIGTERM", echo_state(master2))
    except AssertionError as e:
        print(f"ASSERTION: {e}")
        FAILURES.append("driver raised")
        try:
            os.kill(pid, 9)
        except ProcessLookupError:
            pass
        _, _ = os.waitpid(pid, 0)
    finally:
        try:
            os.close(master)
        except OSError:
            pass

    if FAILURES:
        print(f"\n{len(FAILURES)} failure(s): {', '.join(FAILURES)}")
        sys.exit(1)
    print("\nall e2e checks passed")


if __name__ == "__main__":
    main()
