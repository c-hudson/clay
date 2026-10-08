#!/usr/bin/env python3
"""Turn WebView/native capture pairs into the blind A/B set for the Phase-0 review page:
crop each capture to its output area (no toolbar, status bar or title bar), align the two
vertically by their row-by-row ink profiles, measure them as compare.py does, and shuffle
which one is A with a fixed seed.

    review-pairs.py <capdir> <outdir>

<capdir>/<os>/web/page<N>.png and <capdir>/<os>/native/page<N>.png, for any <os> (mac, win,
linux) and page; a page is used when both sides exist. Captures can be whole screens (the
Windows VM's): the output area is found from Clay's own colours. Writes
<outdir>/img/<os>-<N>-a.png / -b.png and <outdir>/manifest.json (which side is native, plus
row correlation, ink ratio and % pixels differing). Needs Pillow."""
import json
import random
import sys
from pathlib import Path

from PIL import Image

BG, BAR = (19, 25, 38), (40, 75, 99)   # Clay's default output and status-bar colours


def output_area(im):
    """(x0, y0, x1, y1) of the output area: the background band directly above the status
    bar, which is the lowest full-client-width run of the bar colour."""
    im = im.convert("RGB")
    w, h = im.size
    px = im.load()
    bar = None
    for y in range(h - 1, -1, -1):
        x = 0
        while x < w:
            if px[x, y] != BAR:
                x += 1
                continue
            s = x
            while x < w and px[x, y] == BAR:
                x += 1
            if x - s >= w // 4 and (bar is None or x - s > bar[2] - bar[1] - 4):
                bar = (y, s, x)
        if bar and y < bar[0] - 2 and px[bar[1] + 2, y] != BAR:
            break
    if bar is None:
        raise SystemExit("no status bar found")
    ys, xl = bar[0], bar[1]
    while ys > 0 and px[xl + 2, ys - 1] == BAR:
        ys -= 1
    # The bar can have a border line above it: the output ends at the first background row.
    y = ys - 1
    while y > ys - 8 and px[xl + 1, y] != BG:
        y -= 1
    y1 = y + 1
    x1 = xl
    while x1 < w and px[x1, y] == BG:
        x1 += 1
    col = x1 - 3        # the right margin: clear of text, so it reaches up to the chrome
    y0 = y
    while y0 > 0 and px[col, y0 - 1] == BG:
        y0 -= 1
    return xl, y0, x1, y1


def row_ink(im):
    g = im.convert("L")
    w, h = g.size
    d = g.load()
    bg = round(0.299 * BG[0] + 0.587 * BG[1] + 0.114 * BG[2])
    return [sum(max(0, d[x, y] - bg) for x in range(w)) for y in range(h)]


def corr(a, b):
    n = len(a)
    ma, mb = sum(a) / n, sum(b) / n
    num = sum((x - ma) * (y - mb) for x, y in zip(a, b))
    den = (sum((x - ma) ** 2 for x in a) * sum((y - mb) ** 2 for y in b)) ** 0.5 or 1
    return num / den


def main():
    cap, out = Path(sys.argv[1]), Path(sys.argv[2])
    (out / "img").mkdir(parents=True, exist_ok=True)
    rng = random.Random(20261008)
    manifest = []
    for os_dir in sorted(p for p in cap.iterdir() if p.is_dir()):
        pages = sorted(int(p.stem[4:]) for p in (os_dir / "web").glob("page*.png")
                       if (os_dir / "native" / p.name).exists())
        for page in pages:
            web = Image.open(os_dir / "web" / f"page{page}.png").convert("RGB")
            nat = Image.open(os_dir / "native" / f"page{page}.png").convert("RGB")
            wi = web.crop(output_area(web))
            nf = nat.crop(output_area(nat))
            w, h = wi.size
            if nf.size[0] != w or nf.size[1] < h:
                raise SystemExit(f"{os_dir.name} page {page}: output areas {wi.size} vs {nf.size}")
            pw, pn_all = row_ink(wi), row_ink(nf)
            c, off = max((corr(pw, pn_all[o:o + h]), o) for o in range(nf.size[1] - h + 1))
            ni = nf.crop((0, off, w, off + h))
            pn = pn_all[off:off + h]
            a, b = wi.load(), ni.load()
            diff = sum(1 for y in range(h) for x in range(w)
                       if max(abs(a[x, y][k] - b[x, y][k]) for k in range(3)) > 48)
            native_is_a = rng.random() < 0.5
            first, second = (ni, wi) if native_is_a else (wi, ni)
            stem = f"{os_dir.name}-{page}"
            first.save(out / "img" / f"{stem}-a.png", optimize=True)
            second.save(out / "img" / f"{stem}-b.png", optimize=True)
            # Every bench script opens an 800-px-wide window, so the width gives the scale.
            m = dict(os=os_dir.name, page=page, w=w, h=h, scale=round(w / 800),
                     native="a" if native_is_a else "b",
                     corr=round(c, 3), ink=round(sum(pn) / max(1, sum(pw)), 3),
                     diff=round(100 * diff / (w * h), 2))
            manifest.append(m)
            print(m)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=1))


main()
