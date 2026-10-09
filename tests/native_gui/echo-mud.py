#!/usr/bin/env python3
"""A fake MUD that echoes each line back and logs what it received.
   'pw' asks for a password with IAC WILL ECHO (the client masks the next line)."""
import socket, sys, threading
IAC, WILL, WONT, ECHO, GA = 255, 251, 252, 1, 249
port = int(sys.argv[1]); log = sys.argv[2]
state = {"t": 0}
def strip_telnet(d):
    """Drop IAC commands: IAC x, IAC WILL/WONT/DO/DONT opt, IAC SB ... IAC SE.
       (One connection at a time, so one parser state.)"""
    out = bytearray()
    for b in d:
        t = state["t"]
        if t == 0:
            if b == 255: state["t"] = 1
            else: out.append(b)
        elif t == 1:
            if b == 255: out.append(b); state["t"] = 0
            elif 251 <= b <= 254: state["t"] = 2
            elif b == 250: state["t"] = 3
            else: state["t"] = 0
        elif t == 2: state["t"] = 0
        elif t == 3:
            if b == 255: state["t"] = 4
        elif t == 4:
            state["t"] = 0 if b == 240 else 3
    return bytes(out)
def client(c):
    c.sendall(b"Welcome to the echo MUD.\r\n")
    buf = b""; masked = False
    while True:
        d = c.recv(4096)
        if not d:
            return
        buf += strip_telnet(d)
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            line = line.rstrip(b"\r").decode("utf-8", "replace")
            with open(log, "a") as f:
                f.write(line + "\n")
            if masked:
                c.sendall(bytes([IAC, WONT, ECHO]) + b"\r\nPassword accepted.\r\n")
                masked = False
            elif line == "pw":
                c.sendall(b"Password: " + bytes([IAC, WILL, ECHO, IAC, GA]))
                masked = True
            else:
                c.sendall(f"You said: {line}\r\n".encode())
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", port)); s.listen(5)
while True:
    c, _ = s.accept()
    threading.Thread(target=client, args=(c,), daemon=True).start()
