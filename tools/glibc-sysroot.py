#!/usr/bin/env python3
"""Build an Ubuntu 20.04 (glibc 2.31) sysroot for the Linux GUI release asset.

A program linked on this machine needs the glibc this machine has (2.43 in 2026-11),
which shuts out every distribution older than that. Linked against an older glibc it
runs on that one and everything after: Ubuntu 20.04's 2.31 covers Ubuntu 20.04+,
Debian 11+, Linux Mint 20+, Fedora 32+ and RHEL/Rocky/Alma 9+. build-linux-gui.sh
points gcc, the linker and pkg-config at this sysroot.

It holds libc and the libraries the GUI build links or looks up (ROOTS) with their -dev
files, the dependency closure of ROOTS from Ubuntu's focal
release pocket, which never changes. Trust: InRelease is checked with gpgv against
this machine's Ubuntu archive keyring, Packages.xz against InRelease's SHA256, each
.deb against Packages' SHA256 (cached). Absolute symlinks are made relative, so none
leads out of the sysroot into this machine's newer libraries.

Usage: tools/glibc-sysroot.py [SYSROOT]
  SYSROOT defaults to ~/.cache/clay-glibc-sysroot/focal-<hash of ROOTS and EXCLUDE>; a
  sysroot made from another package list is removed. The path changes with the list
  because a -sys crate's build script reruns only when its environment changes: one that
  fell back to a bundled C library while a package was missing would keep doing so.
  The .debs are cached beside it. Prints the sysroot's path last.
"""
import hashlib, lzma, os, re, shutil, subprocess, sys, tempfile, urllib.request

MIRROR = "http://archive.ubuntu.com/ubuntu"   # plain http: integrity comes from gpgv
DIST, COMPONENT, ARCH = "focal", "main", "amd64"
KEYRING = "/usr/share/keyrings/ubuntu-archive-keyring.gpg"
# libc, the libraries the binary links (ldd: xcb, xkbcommon, xkbcommon-x11, ALSA), the
# ones a -sys crate's build script asks pkg-config for even where the link then drops them
# (fontconfig, freetype, wayland), and libstdc++: freetype-sys takes a system FreeType only
# from 2.12 (focal has 2.10) and otherwise compiles its own as C++, linking -lstdc++.
ROOTS = ["libc6-dev", "libgcc-s1", "libstdc++6", "libxcb1-dev", "libxcb-xkb-dev",
         "libxkbcommon-dev", "libxkbcommon-x11-dev", "libasound2-dev", "libfontconfig1-dev",
         "libfreetype6-dev", "libwayland-dev"]
# The -dev links for gcc's own libraries, which their focal -dev packages keep in gcc-10's
# directory: without them -lgcc_s and -lstdc++ find this machine's gcc's copies, which
# need a newer glibc.
DEV_LINKS = {"lib/x86_64-linux-gnu/libgcc_s.so": "libgcc_s.so.1",
             "usr/lib/x86_64-linux-gnu/libstdc++.so": "libstdc++.so.6"}
# Dependencies no link needs, which pull in perl, glib, ICU and fonts (160 MB of 220): the
# config, tool and data packages of the roots above (pkg-config is this machine's own).
EXCLUDE = {"fontconfig-config", "libwayland-bin", "debconf", "ucf", "perl-base", "dpkg",
           "coreutils", "tar", "tzdata", "sensible-utils", "pkg-config"}


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "Debian APT-HTTP/1.3"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def verified_release():
    """InRelease's signed text, or exit when the signature doesn't verify."""
    for tool in ("gpgv", "dpkg-deb"):
        if not shutil.which(tool):
            sys.exit(f"error: {tool} not found")
    if not os.path.exists(KEYRING):
        sys.exit(f"error: {KEYRING} not found (the ubuntu-keyring package)")
    raw = fetch(f"{MIRROR}/dists/{DIST}/InRelease")
    with tempfile.TemporaryDirectory() as tmp:
        src, out = os.path.join(tmp, "InRelease"), os.path.join(tmp, "Release")
        with open(src, "wb") as f:
            f.write(raw)
        r = subprocess.run(["gpgv", "--keyring", KEYRING, "--output", out, src],
                           capture_output=True, text=True)
        if r.returncode != 0:
            sys.exit(f"error: {DIST} InRelease does not verify:\n{r.stderr}")
        return open(out).read()


def release_sha256(release, path):
    in_sha = False
    for line in release.splitlines():
        if not line.startswith(" "):
            in_sha = line.startswith("SHA256:")
        elif in_sha:
            digest, _size, name = line.split()
            if name == path:
                return digest
    sys.exit(f"error: {path} is not listed in {DIST}'s InRelease")


def parse_index(text):
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
            pkgs.setdefault(fields["Package"], fields)
    return pkgs


def dep_names(field):
    """'a (>= 1), b | c' -> [['a'], ['b', 'c']] (alternatives per clause)."""
    out = []
    for clause in filter(None, (c.strip() for c in (field or "").split(","))):
        out.append([re.sub(r"\s*\(.*?\)|:\w+", "", alt).strip() for alt in clause.split("|")])
    return out


def relativize_links(sysroot):
    """Point every absolute symlink at the same path inside the sysroot."""
    for dirpath, dirnames, filenames in os.walk(sysroot):
        for name in dirnames + filenames:
            p = os.path.join(dirpath, name)
            if os.path.islink(p):
                target = os.readlink(p)
                if target.startswith("/"):
                    os.unlink(p)
                    os.symlink(os.path.relpath(sysroot + target, dirpath), p)


def main():
    if len(sys.argv) > 1:
        sysroot = os.path.abspath(os.path.expanduser(sys.argv[1]))
    else:
        parent = os.path.expanduser("~/.cache/clay-glibc-sysroot")
        tag = hashlib.sha256(repr((DIST, ROOTS, sorted(EXCLUDE))).encode()).hexdigest()[:8]
        sysroot = os.path.join(parent, f"{DIST}-{tag}")
        for old in os.listdir(parent) if os.path.isdir(parent) else ():
            if old != os.path.basename(sysroot) and (old == DIST or old.startswith(DIST + "-")):
                shutil.rmtree(os.path.join(parent, old))
    cache = os.path.join(os.path.dirname(sysroot), f".debs-{DIST}")
    os.makedirs(cache, exist_ok=True)
    os.makedirs(sysroot, exist_ok=True)

    release = verified_release()
    path = f"{COMPONENT}/binary-{ARCH}/Packages.xz"
    packed = fetch(f"{MIRROR}/dists/{DIST}/{path}")
    if hashlib.sha256(packed).hexdigest() != release_sha256(release, path):
        sys.exit(f"error: SHA256 mismatch for {path}")
    index = parse_index(lzma.decompress(packed).decode())
    provides = {}
    for name, f in index.items():
        for alts in dep_names(f.get("Provides")):
            for p in alts:
                provides.setdefault(p, name)

    def resolve(name):
        return name if name in index else provides.get(name)

    want, queue = set(), list(ROOTS)
    while queue:
        name = resolve(queue.pop())
        if not name or name in want or name in EXCLUDE:
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
            data = fetch(f"{MIRROR}/{f['Filename']}")
            if hashlib.sha256(data).hexdigest() != f.get("SHA256"):
                sys.exit(f"error: SHA256 mismatch for {f['Filename']}")
            with open(deb + ".part", "wb") as out:
                out.write(data)
            os.replace(deb + ".part", deb)
        subprocess.run(["dpkg-deb", "-x", deb, sysroot], check=True)
        with open(stamp, "w") as s:
            s.write(f["Version"])
        fetched += 1
    relativize_links(sysroot)
    for link, target in DEV_LINKS.items():
        p = os.path.join(sysroot, link)
        if not os.path.lexists(p):
            os.symlink(target, p)

    print(f"{len(want)} packages ({fetched} newly unpacked)", file=sys.stderr)
    print(sysroot)


if __name__ == "__main__":
    main()
