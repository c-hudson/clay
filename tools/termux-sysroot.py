#!/usr/bin/env python3
"""Build a Termux aarch64 sysroot from Termux's own apt packages.

Termux cross-compiles every package it ships with the Android NDK, so its .deb files
are exactly what an NDK cross-compile of Clay's webview GUI needs to link against
(GTK3, WebKit2GTK, libsoup, glib, X11, libc++_shared). This resolves the dependency
closure of the root packages below from the termux-main and termux-x11 indexes,
downloads each .deb (SHA256-checked, cached) and unpacks it with `dpkg-deb -x`.

Usage: tools/termux-sysroot.py [SYSROOT]
  SYSROOT defaults to ~/.cache/clay-termux-sysroot. Libraries land under
  SYSROOT/data/data/com.termux/files/usr. Re-running only fetches packages whose
  version changed. Prints the sysroot path on the last line.
"""
import hashlib, os, re, subprocess, sys, urllib.request

ARCH = "aarch64"
MIRROR = "https://packages-cf.termux.dev/apt"
# (repo base, dist, component)
REPOS = [
    (f"{MIRROR}/termux-main", "stable", "main"),
    (f"{MIRROR}/termux-x11", "x11", "main"),
]
ROOTS = ["webkit2gtk-4.1", "gtk3", "libc++"]


def fetch(url):
    # The CDN rejects urllib's default User-Agent with a 403.
    req = urllib.request.Request(url, headers={"User-Agent": "Debian APT-HTTP/1.3"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def parse_index(text, base):
    pkgs = {}
    for stanza in text.split("\n\n"):
        fields, key = {}, None
        for line in stanza.splitlines():
            if line.startswith((" ", "\t")) and key:
                fields[key] += "\n" + line.strip()
            elif ":" in line:
                key, _, val = line.partition(":")
                fields[key] = val.strip()
        if "Package" in fields and "Filename" in fields:
            fields["_base"] = base
            pkgs.setdefault(fields["Package"], fields)
    return pkgs


def dep_names(field):
    """'a (>= 1), b | c' -> [['a'], ['b', 'c']] (alternatives per clause)."""
    out = []
    for clause in filter(None, (c.strip() for c in (field or "").split(","))):
        out.append([re.sub(r"\s*\(.*?\)|:\w+", "", alt).strip() for alt in clause.split("|")])
    return out


def main():
    sysroot = os.path.abspath(os.path.expanduser(
        sys.argv[1] if len(sys.argv) > 1 else "~/.cache/clay-termux-sysroot"))
    cache = os.path.join(sysroot, ".debs")
    os.makedirs(cache, exist_ok=True)

    index, provides = {}, {}
    for base, dist, comp in REPOS:
        text = fetch(f"{base}/dists/{dist}/{comp}/binary-{ARCH}/Packages").decode()
        for name, f in parse_index(text, base).items():
            index.setdefault(name, f)
    for name, f in index.items():
        for alts in dep_names(f.get("Provides")):
            for p in alts:
                provides.setdefault(p, name)

    def resolve(name):
        if name in index:
            return name
        return provides.get(name)

    want, queue = set(), list(ROOTS)
    while queue:
        name = resolve(queue.pop())
        if not name or name in want:
            continue
        want.add(name)
        f = index[name]
        for alts in dep_names(f.get("Depends")) + dep_names(f.get("Pre-Depends")):
            hit = next((resolve(a) for a in alts if resolve(a)), None)
            if hit:
                queue.append(hit)
            else:
                print(f"warning: {name}: unresolved dependency {alts}", file=sys.stderr)

    stamp_dir = os.path.join(sysroot, ".installed")
    os.makedirs(stamp_dir, exist_ok=True)
    fetched = 0
    for name in sorted(want):
        f = index[name]
        stamp = os.path.join(stamp_dir, name)
        if os.path.exists(stamp) and open(stamp).read() == f["Version"]:
            continue
        deb = os.path.join(cache, os.path.basename(f["Filename"]))
        if not os.path.exists(deb):
            data = fetch(f"{f['_base']}/{f['Filename']}")
            if hashlib.sha256(data).hexdigest() != f.get("SHA256"):
                sys.exit(f"error: SHA256 mismatch for {f['Filename']}")
            with open(deb + ".part", "wb") as out:
                out.write(data)
            os.replace(deb + ".part", deb)
        subprocess.run(["dpkg-deb", "-x", deb, sysroot], check=True)
        with open(stamp, "w") as s:
            s.write(f["Version"])
        fetched += 1

    print(f"{len(want)} packages ({fetched} newly unpacked)", file=sys.stderr)
    print(sysroot)


if __name__ == "__main__":
    main()
