#!/usr/bin/env python3
"""Compare text weight between two xwd captures: the 'ink' (brightness above the
background) per pixel row over a column range, aligned by cross-correlation.
Usage: ink.py web.xwd native.xwd x0 x1 web_y0 web_y1"""
import sys

sys.path.insert(0, __import__("os").path.dirname(__file__))
from xwd2png import read_xwd  # noqa: E402


def profile(path, x0, x1):
    w, h, rows = read_xwd(path)
    bg = rows[h // 2][(w - 2) * 3:(w - 1) * 3]
    bgl = 0.2126 * bg[0] + 0.7152 * bg[1] + 0.0722 * bg[2]
    prof = []
    for r in rows:
        s = 0.0
        for x in range(x0, x1):
            l = 0.2126 * r[x * 3] + 0.7152 * r[x * 3 + 1] + 0.0722 * r[x * 3 + 2]
            s += max(0.0, l - bgl)
        prof.append(s)
    return prof


web, nat = sys.argv[1], sys.argv[2]
x0, x1, wy0, wy1 = map(int, sys.argv[3:7])
pw, pn = profile(web, x0, x1), profile(nat, x0, x1)
seg = pw[wy0:wy1]
best = None
for off in range(-80, 80):
    ny0 = wy0 + off
    if ny0 < 0 or ny0 + len(seg) > len(pn):
        continue
    cand = pn[ny0:ny0 + len(seg)]
    # correlation of shapes (normalised)
    ma, mb = sum(seg) / len(seg), sum(cand) / len(cand)
    num = sum((a - ma) * (b - mb) for a, b in zip(seg, cand))
    den = (sum((a - ma) ** 2 for a in seg) * sum((b - mb) ** 2 for b in cand)) ** 0.5 or 1
    c = num / den
    if best is None or c > best[0]:
        best = (c, off)
c, off = best
nat_seg = pn[wy0 + off:wy0 + off + len(seg)]
print(f"offset {off:+d}px  correlation {c:.3f}  ink web {sum(seg):.0f}  native {sum(nat_seg):.0f}  ratio {sum(nat_seg) / max(1, sum(seg)):.3f}")
