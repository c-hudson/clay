#!/usr/bin/env python3
"""Convert an `xwd -silent` capture (32-bit TrueColor ZPixmap) to PNG, with nothing but
the standard library. Optionally crop: xwd2png.py in.xwd out.png [x y w h]"""
import struct
import sys
import zlib


def read_xwd(path):
    data = open(path, "rb").read()
    header_size, version = struct.unpack(">II", data[:8])
    fields = struct.unpack(">25I", data[:100])
    (_hs, _ver, pixmap_format, depth, width, height, _xoff, byte_order, _unit, _bit_order,
     _pad, bits_per_pixel, bytes_per_line, _visual_class, red_mask, green_mask, blue_mask,
     _bits_rgb, _cmap_entries, ncolors, _ww, _wh, _wx, _wy, _bw) = fields
    if version != 7 or pixmap_format != 2 or bits_per_pixel not in (24, 32):
        raise SystemExit(f"unsupported xwd: version {version} format {pixmap_format} bpp {bits_per_pixel}")
    offset = header_size + ncolors * 12
    pixels = data[offset:offset + bytes_per_line * height]
    step = bits_per_pixel // 8
    if bits_per_pixel == 24 and bytes_per_line >= width * 4:
        # A depth-24 capture that says 24 bits per pixel with room for 4 bytes a pixel is
        # one or the other: a window's own dump packs 4 (the top byte unused, always 0),
        # a root dump under a window manager packs 3 and pads the row (found 2026-11-10:
        # read as 4, the picture came out three quarters wide and striped). The first row
        # with anything in it tells: all-zero fourth bytes mean 4.
        step = 3
        for y in range(height):
            row = pixels[y * bytes_per_line:y * bytes_per_line + width * 4]
            if any(row):
                step = 4 if not any(row[3::4]) else 3
                break

    def shift(mask):
        return (mask & -mask).bit_length() - 1

    rs, gs, bs = shift(red_mask), shift(green_mask), shift(blue_mask)
    fmt = "<I" if byte_order == 0 else ">I"
    rows = []
    for y in range(height):
        row = bytearray()
        base = y * bytes_per_line
        for x in range(width):
            if step == 4:
                (p,) = struct.unpack_from(fmt, pixels, base + x * 4)
            else:
                p = int.from_bytes(pixels[base + x * 3:base + x * 3 + 3], "little" if byte_order == 0 else "big")
            row += bytes(((p & red_mask) >> rs, (p & green_mask) >> gs, (p & blue_mask) >> bs))
        rows.append(bytes(row))
    return width, height, rows


def write_png(path, width, height, rows):
    def chunk(kind, body):
        c = struct.pack(">I", len(body)) + kind + body
        return c + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + r for r in rows)
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
    open(path, "wb").write(png)


if __name__ == "__main__":
    w, h, rows = read_xwd(sys.argv[1])
    if len(sys.argv) >= 7:
        x, y, cw, ch = map(int, sys.argv[3:7])
        rows = [r[x * 3:(x + cw) * 3] for r in rows[y:y + ch]]
        w, h = cw, ch
    if len(sys.argv) == 8:
        # Integer zoom, nearest neighbour, for inspecting pixels.
        z = int(sys.argv[7])
        rows = [b"".join(r[i:i + 3] * z for i in range(0, len(r), 3)) for r in rows for _ in range(z)]
        w, h = w * z, h * z
    write_png(sys.argv[2], w, h, rows)
    print(f"{sys.argv[2]}: {w}x{h}")
