#!/usr/bin/env python3
"""Talk QMP over a unix socket: send a screendump to a host-side file."""
import json
import socket
import sys

sock_path = sys.argv[1]
out_file = sys.argv[2]

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sock_path)
s.settimeout(5)


def recv_obj():
    buf = b""
    while True:
        chunk = s.recv(4096)
        if not chunk:
            raise RuntimeError(f"qmp closed; buf={buf!r}")
        buf += chunk
        try:
            return json.loads(buf)
        except json.JSONDecodeError:
            continue


greeting = recv_obj()  # {"QMP": {...}}
if "QMP" not in greeting:
    raise RuntimeError(f"unexpected greeting: {greeting}")

s.sendall(json.dumps({"execute": "qmp_capabilities"}).encode() + b"\n")
recv_obj()

s.sendall(
    json.dumps(
        {"execute": "screendump", "arguments": {"filename": out_file}}
    ).encode()
    + b"\n"
)
r = recv_obj()
print(r)
if "return" not in r:
    sys.exit(1)
