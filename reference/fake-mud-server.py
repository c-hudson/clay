# Minimal MUD that emits numbered lines with a trailing prompt, plus blank/ANSI-only lines.
import socket, threading, time, sys
port = int(sys.argv[1])
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.1', port)); s.listen(5)
print(f"mud listening on {port}", flush=True)
def handle(c):
    try:
        n = 0
        for burst in range(40):
            chunk = b''
            for _ in range(5):
                n += 1
                chunk += f"MUDLINE {n:04d} the quick brown fox\r\n".encode()
            chunk += b"\x1b[0m\r\n"                       # ANSI-only line
            chunk += b"\r\n"                              # blank line
            chunk += f"HP:100 MP:50 [{n}]> ".encode()     # trailing partial prompt
            c.sendall(chunk)
            time.sleep(0.15)
        time.sleep(30)
    except Exception as e:
        print("mud conn ended:", e, flush=True)
while True:
    c, _ = s.accept()
    threading.Thread(target=handle, args=(c,), daemon=True).start()
