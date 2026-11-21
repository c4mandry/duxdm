#!/usr/bin/env python3
"""Drive a QEMU serial console exposed as a unix socket (server mode).

Usage: cons.py SOCKET CMD_FILE [TIMEOUT_SECONDS]

CMD_FILE lines (comments start with #):
  expect <text>   wait until <text> appears in the console output
  send <text>     write <text> + newline to the console
  sleep <secs>    wait <secs> seconds

Prints the full console transcript; exits 0 only if every step ran.
"""
import select
import socket
import sys
import time


def main():
    sock_path, cmd_file = sys.argv[1], sys.argv[2]
    timeout = float(sys.argv[3]) if len(sys.argv) > 3 else 300.0

    steps = [
        l.strip()
        for l in open(cmd_file, encoding="utf-8")
        if l.strip() and not l.strip().startswith("#")
    ]

    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    for _ in range(100):  # wait for qemu to create the socket
        try:
            s.connect(sock_path)
            break
        except (FileNotFoundError, ConnectionRefusedError):
            time.sleep(0.2)
    else:
        print("could not connect to console socket", file=sys.stderr)
        sys.exit(1)
    s.setblocking(False)

    buf = b""
    ci = 0
    deadline = time.time() + timeout
    while ci < len(steps) and time.time() < deadline:
        r, _, _ = select.select([s], [], [], 0.1)
        if r:
            try:
                chunk = s.recv(65536)
            except BlockingIOError:
                chunk = b""
            if not chunk:
                break
            buf += chunk
            sys.stdout.buffer.write(chunk)
            sys.stdout.buffer.flush()

        line = steps[ci]
        if line.startswith("expect "):
            if line[7:].encode() in buf:
                ci += 1
                buf = b""
        elif line.startswith("raw "):
            # raw bytes, \xHH escapes decoded, no trailing newline
            payload = line[4:]
            out = b""
            i = 0
            while i < len(payload):
                if payload[i] == "\\" and i + 3 < len(payload) and payload[i + 1] == "x":
                    out += bytes([int(payload[i + 2 : i + 4], 16)])
                    i += 4
                else:
                    out += payload[i].encode()
                    i += 1
            s.sendall(out)
            buf = b""  # only match output produced AFTER this send
            ci += 1
        elif line.startswith("send "):
            s.sendall(line[5:].encode() + b"\n")
            buf = b""  # only match output produced AFTER this send
            ci += 1
        elif line.startswith("sleep "):
            time.sleep(float(line[6:]))
            ci += 1
        else:
            ci += 1

    ok = ci >= len(steps)
    print(f"\n[DONE ok={ok}]")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
