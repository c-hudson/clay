"""Minimal WebSocket client that speaks Clay's protocol far enough to verify the
delivered-seq contract end to end against a real running daemon."""
import socket, base64, os, json, struct, sys, time, threading, collections

class WS:
    def __init__(self, host, port, path='/ws'):
        self.s = socket.create_connection((host, port), timeout=10)
        key = base64.b64encode(os.urandom(16)).decode()
        req = (f"GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\n"
               f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n")
        self.s.sendall(req.encode())
        buf = b''
        while b'\r\n\r\n' not in buf:
            d = self.s.recv(4096)
            if not d: raise RuntimeError('handshake eof: ' + repr(buf))
            buf += d
        head, _, rest = buf.partition(b'\r\n\r\n')
        if b'101' not in head.split(b'\r\n')[0]:
            raise RuntimeError('handshake failed: ' + head.decode(errors='replace')[:400])
        self.buf = rest

    def _recv_exact(self, n):
        while len(self.buf) < n:
            d = self.s.recv(65536)
            if not d: raise RuntimeError('eof')
            self.buf += d
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def recv(self):
        while True:
            h = self._recv_exact(2)
            fin = h[0] & 0x80; op = h[0] & 0x0f
            ln = h[1] & 0x7f
            if ln == 126: ln = struct.unpack('>H', self._recv_exact(2))[0]
            elif ln == 127: ln = struct.unpack('>Q', self._recv_exact(8))[0]
            payload = self._recv_exact(ln)
            if op == 8: raise RuntimeError('closed by peer')
            if op == 9:  # ping
                self.send_frame(payload, 0xA); continue
            if op in (1, 2, 0):
                return payload.decode('utf-8', 'replace')

    def send_frame(self, data, op=1):
        mask = os.urandom(4)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        n = len(data)
        if n < 126: hdr = struct.pack('!BB', 0x80 | op, 0x80 | n)
        elif n < 65536: hdr = struct.pack('!BBH', 0x80 | op, 0x80 | 126, n)
        else: hdr = struct.pack('!BBQ', 0x80 | op, 0x80 | 127, n)
        self.s.sendall(hdr + mask + masked)

    def send(self, obj):
        self.send_frame(json.dumps(obj).encode())

def main():
    port = int(sys.argv[1]); password = sys.argv[2]; duration = float(sys.argv[3])
    ws = WS('127.0.0.1', port)
    import hashlib
    hello = json.loads(ws.recv())
    print('HELLO', hello)
    challenge = hello.get('challenge')
    pw_sha = hashlib.sha256(password.encode()).hexdigest()
    if challenge:
        ph = hashlib.sha256((pw_sha + challenge).encode()).hexdigest()
        ws.send({'type': 'AuthRequest', 'password_hash': ph, 'challenge_response': True,
                 'client_uid': 'verify-client-1', 'resume': []})
    else:
        ws.send({'type': 'AuthRequest', 'password_hash': pw_sha,
                 'client_uid': 'verify-client-1', 'resume': []})

    seen = set()          # every seq the server delivered
    lines_by_seq = {}
    initial_seqs = set()
    authed = False
    connected_sent = False
    t0 = time.time()
    counts = collections.Counter()
    while time.time() - t0 < duration:
        try:
            ws.s.settimeout(max(0.2, duration - (time.time() - t0)))
            raw = ws.recv()
        except socket.timeout:
            continue
        except RuntimeError as e:
            print('recv ended:', e); break
        msg = json.loads(raw)
        t = msg.get('type')
        counts[t] += 1
        if t == 'AuthResponse':
            print('AUTH', msg.get('success'), msg.get('message'))
            authed = bool(msg.get('success'))
        elif t == 'InitialState':
            for w in msg.get('worlds', []):
                for l in (w.get('output_lines_ts') or []):
                    if l.get('seq') is not None:
                        seen.add(l['seq']); initial_seqs.add(l['seq'])
                        lines_by_seq[l['seq']] = l['text']
            print('INITIALSTATE worlds=%d seeded_seqs=%d' % (len(msg.get('worlds', [])), len(initial_seqs)))
            if not connected_sent:
                ws.send({'type': 'UpdateViewState', 'world_index': 0, 'visible_lines': 200, 'visible_columns': 200})
                ws.send({'type': 'ConnectWorld', 'world_index': 0})
                connected_sent = True
        elif t == 'ServerData':
            seq = msg.get('seq'); end = msg.get('end_seq')
            data = msg.get('data', '')
            body = data[:-1] if data.endswith('\n') else data
            lines = body.split('\n') if body != '' else ['']
            if msg.get('flush'):
                # Same contract as app.js: a flush wipes the client's delivered-seq record,
                # because the server's buffer (and its seq space) was reset underneath us.
                print('FLUSH at seq', seq, '- resetting delivered-seq record')
                seen.clear(); lines_by_seq.clear()
            real = (seq is not None and (seq > 0 or end is not None))
            if not real:
                continue
            end = end if end is not None else seq + len(lines) - 1
            span = end - seq + 1
            if span != len(lines):
                print('!! SPAN MISMATCH seq=%s end=%s span=%d lines=%d data=%r' % (seq, end, span, len(lines), data[:120]))
            for i, l in enumerate(lines):
                s = seq + i
                if s in lines_by_seq and lines_by_seq[s] != l:
                    print('!! SEQ %d REFILED: had %r now %r' % (s, lines_by_seq[s][:50], l[:50]))
                lines_by_seq.setdefault(s, l)
            for s in range(seq, end + 1):
                seen.add(s)
        elif t == 'ResyncRequired':
            print('RESYNC-REQUIRED', msg)
        elif t == 'PendingLinesUpdate':
            if msg.get('count'):
                ws.send({'type': 'ReleasePending', 'world_index': msg.get('world_index', 0), 'count': 0})
        elif t == 'PingCheck':
            acked = []
            if seen:
                lo = min(seen); f = lo
                while f + 1 in seen: f += 1
                acked = [[0, f]]
            ws.send({'type': 'PongCheck', 'nonce': msg.get('nonce'), 'acked': acked})
    # Report
    print('MSG COUNTS', dict(counts))
    if seen:
        lo, hi = min(seen), max(seen)
        holes = [s for s in range(lo, hi + 1) if s not in seen]
        print('SEQ RANGE %d..%d  delivered=%d  holes=%d' % (lo, hi, len(seen), len(holes)))
        if holes:
            print('HOLES:', holes[:40])
    else:
        print('NO SEQS DELIVERED')
    # dump what we think the world looks like, tail
    tail = [lines_by_seq[k] for k in sorted(lines_by_seq)[-12:]]
    print('CLIENT TAIL:')
    for l in tail: print('   ', repr(l[:80]))

main()
