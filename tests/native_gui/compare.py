#!/usr/bin/env python3
"""Compare a WebView capture with a native one (PNG, same scale): align the two vertically
by their ink profiles, then report the ink ratio (text weight) and how many pixels differ.
Standard library only.
    compare.py web.png native.png x0 x1 web_y0 web_y1 [diff.png]
The diff image (optional) shows the web capture with differing pixels in red."""
import struct
import sys
import zlib


def read_png(path):
    data = open(path, "rb").read()
    pos, idat = 8, b""
    while pos < len(data):
        (ln,) = struct.unpack(">I", data[pos:pos + 4])
        kind, body = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + ln]
        pos += 12 + ln
        if kind == b"IHDR":
            w, h, _bd, ctype = struct.unpack(">IIBB", body[:10])
        elif kind == b"IDAT":
            idat += body
    raw = zlib.decompress(idat)
    bpp = 4 if ctype == 6 else 3
    stride, prev, i, rows = w * bpp, bytearray(w * bpp), 0, []
    for _ in range(h):
        f = raw[i]
        i += 1
        line = bytearray(raw[i:i + stride])
        i += stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + ((a + b) >> 1)) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line))
        prev = line
    if bpp == 4:
        rows = [bytes(r[j] for j in range(len(r)) if j % 4 != 3) for r in rows]
    return w, h, rows


def write_png(path, w, h, rows):
    def chunk(k, b):
        return struct.pack(">I", len(b)) + k + b + struct.pack(">I", zlib.crc32(k + b) & 0xFFFFFFFF)
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(b"".join(b"\x00" + r for r in rows), 6)) + chunk(b"IEND", b""))


def lum(r, x):
    return 0.2126 * r[x * 3] + 0.7152 * r[x * 3 + 1] + 0.0722 * r[x * 3 + 2]


def main():
    web, nat = sys.argv[1], sys.argv[2]
    x0, x1, wy0, wy1 = map(int, sys.argv[3:7])
    _, _, wr = read_png(web)
    _, nh, nr = read_png(nat)
    bg = lum(wr[wy1 - 1], x1 - 1)
    prof = lambda rows, y: sum(max(0.0, lum(rows[y], x) - bg) for x in range(x0, x1))
    pw = [prof(wr, y) for y in range(wy0, wy1)]
    best = None
    for off in range(-200, 200):
        ny0 = wy0 + off
        if ny0 < 0 or ny0 + len(pw) > nh:
            continue
        pn = [prof(nr, ny0 + k) for k in range(len(pw))] if best is None or abs(off) < 400 else None
        ma, mb = sum(pw) / len(pw), sum(pn) / len(pn)
        num = sum((a - ma) * (b - mb) for a, b in zip(pw, pn))
        den = (sum((a - ma) ** 2 for a in pw) * sum((b - mb) ** 2 for b in pn)) ** 0.5 or 1
        if best is None or num / den > best[0]:
            best = (num / den, off, pn)
    corr, off, pn = best
    diff_px = total = 0
    out = []
    for k, y in enumerate(range(wy0, wy1)):
        a, b = wr[y], nr[y + off]
        row = bytearray(a[x0 * 3:x1 * 3])
        for x in range(x0, x1):
            total += 1
            if max(abs(a[x * 3 + c] - b[x * 3 + c]) for c in range(3)) > 48:
                diff_px += 1
                row[(x - x0) * 3:(x - x0) * 3 + 3] = b"\xff\x20\x20"
        out.append(bytes(row))
    print(f"offset {off:+d}px  correlation {corr:.3f}  ink ratio {sum(pn) / max(1, sum(pw)):.3f}  "
          f"pixels differing {100.0 * diff_px / total:.2f}%")
    if len(sys.argv) > 7:
        write_png(sys.argv[7], x1 - x0, wy1 - wy0, out)


main()
