#!/usr/bin/env python3
"""Delete cargo build artifacts that the next build will not use.

cargo never removes anything from target/: every dependency version, feature set,
toolchain and Clay version it has built stays in deps/, build/ and incremental/
(target/ had reached 188 GB by v1.7.16). This keeps what the next build of any
configuration in recent use will reuse and deletes the rest, so no build gets slower.

Per CARGO_TARGET_DIR (TARGET_DIR itself and each subdir with its own
.rustc_info.json: target/native-gui, target/termux-gui, ...):
  kept   - every unit that a Clay unit built in the last --days days reaches
           through its fingerprint's `deps`, host build scripts and proc-macros
           included (a cross build keeps those in target/<profile>, beside its own
           target/<triple>/<profile>). Every recent Clay unit is a root, not just
           the newest per feature set: `cargo build` and `cargo test` build Clay
           with the same features against different dependencies (dev-dependencies
           widen tokio's features), so "newest" would drop the other one's. Also
           kept: anything built in the last day, so the finished dependencies of
           an interrupted build, which no Clay unit names yet, survive.
  pruned - every other unit: .fingerprint/NAME-HASH, deps/*-HASH*, build/NAME-HASH,
           examples/*-HASH*, and any such file whose hash no fingerprint claims;
         - Clay's own artifacts built before the last version bump: the version is
           in their hash, so the next build compiles Clay under new names anyway.
           Their fingerprint dirs stay, since those are what keep the
           dependencies alive until the configuration is rebuilt or ages out;
         - incremental caches last written before the version bump or the window.
Final binaries (target/debug/clay, ...) are never touched.

A target dir is pruned only while holding cargo's own locks on it, so a build
running alongside is never pulled out from under; a locked dir is skipped.

Usage: tools/prune-target.py [--dry-run] [--list] [--days N] [TARGET_DIR]
  TARGET_DIR defaults to ./target. Run from the repo root: the package names come
  from Cargo.toml and the version bump's time from `git log`.
"""
import argparse, json, os, re, shutil, subprocess, sys, time
from collections import defaultdict, namedtuple

try:
    import fcntl
except ImportError:  # Windows
    fcntl = None
    import msvcrt

HASH = re.compile(r"-([0-9a-f]{16})(?:\.|$)")
Unit = namedtuple("Unit", "val pd pkg hash kind mtime j")


def workspace_packages():
    # [package] name, read without tomllib: the Mac VM's python3 is 3.9.
    section = None
    with open("Cargo.toml") as f:
        for line in f:
            s = line.strip()
            if s.startswith("["):
                section = s
            elif section == "[package]":
                m = re.match(r'name\s*=\s*"([^"]+)"', s)
                if m:
                    return {m.group(1)}
    return set()


def version_bump_time():
    # The last commit that changed the package version line; 0 (prune nothing on
    # that ground) when git can't say.
    try:
        out = subprocess.run(["git", "log", "-1", "--format=%ct", "-G", r'^version *= *"',
                              "--", "Cargo.toml"], capture_output=True, text=True, check=True)
        return int(out.stdout.strip() or 0)
    except (OSError, subprocess.CalledProcessError, ValueError):
        return 0


def target_dirs(root):
    yield root
    for d in sorted(os.listdir(root)):
        if os.path.isfile(os.path.join(root, d, ".rustc_info.json")):
            yield os.path.join(root, d)


def profile_dirs(tdir):
    # tdir/<profile> and tdir/<triple>/<profile>, never a nested target dir.
    out = []
    for d in sorted(os.listdir(tdir)):
        p = os.path.join(tdir, d)
        if not os.path.isdir(p) or os.path.isfile(os.path.join(p, ".rustc_info.json")):
            continue
        for q in [p] + [os.path.join(p, e) for e in sorted(os.listdir(p))]:
            if os.path.isdir(os.path.join(q, ".fingerprint")):
                out.append(q)
    return out


def lock(pdirs):
    """Take every cargo lock in these profile dirs, or none (returns None) if any is held.
    cargo flock()s these on Unix and LockFileEx()es their whole range on Windows, which
    a lock on their first byte conflicts with."""
    fds = []
    for pd in pdirs:
        for f in os.listdir(pd):
            if f.startswith(".cargo") and f.endswith("lock"):
                fd = os.open(os.path.join(pd, f), os.O_RDWR)
                try:
                    if fcntl:
                        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    else:
                        msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
                except OSError:
                    os.close(fd)
                    unlock(fds)
                    return None
                fds.append(fd)
    return fds


def unlock(fds):
    for fd in fds:
        if not fcntl:
            msvcrt.locking(fd, msvcrt.LK_UNLCK, 1)
        os.close(fd)


def load_units(pdirs):
    """Every unit, by its fingerprint value (what a dependent's `deps` names). A value
    is not unique: two build-script runs with the same inputs share one, though their
    outputs live under different hashes, and a dependent names only the value - so
    every unit with it must be kept."""
    units = defaultdict(list)
    for pd in pdirs:
        fp = os.path.join(pd, ".fingerprint")
        for name in os.listdir(fp):
            m = HASH.search(name)
            d = os.path.join(fp, name)
            if not m or not os.path.isdir(d):
                continue
            for f in os.listdir(d):
                jp = os.path.join(d, f + ".json")
                if f.endswith(".json") or not os.path.isfile(jp):
                    continue
                try:
                    with open(os.path.join(d, f)) as h:
                        # cargo writes the u64 as hex of its little-endian bytes
                        val = int.from_bytes(bytes.fromhex(h.read().strip()), "little")
                    with open(jp) as h:
                        j = json.load(h)
                except (OSError, ValueError):
                    continue
                units[val].append(Unit(val, pd, name[:m.start()], m.group(1), f,
                                       os.path.getmtime(jp), j))
    return units


def files_under(p):
    if not os.path.isdir(p) or os.path.islink(p):
        return [p]
    return [os.path.join(dp, f) for dp, _, fns in os.walk(p) for f in fns]


def remove(p):
    if os.path.isdir(p) and not os.path.islink(p):
        shutil.rmtree(p)
    else:
        os.remove(p)


def prune(tdir, packages, bump, since, dry_run, listing):
    """Returns ({profile dir: [artifact bytes, incremental bytes]}, paths it failed to
    delete), or a reason it was skipped."""
    pdirs = profile_dirs(tdir)
    if not pdirs:
        return {}, 0
    held = lock(pdirs)
    if held is None:
        return "skipped: a cargo build holds its lock"
    try:
        units = load_units(pdirs)
        recent = time.time() - 86400
        live, stack = set(), [u.val for us in units.values() for u in us
                              if u.mtime >= recent or u.pkg in packages and u.mtime >= since]
        while stack:
            v = stack.pop()
            if v not in live:
                live.add(v)
                for u in units.get(v, ()):
                    stack.extend(dep[-1] for dep in u.j.get("deps", []))

        keep_fp = {pd: set() for pd in pdirs}
        keep_art = {pd: set() for pd in pdirs}
        live_units = [u for v in live for u in units.get(v, ())]
        for u in live_units:
            keep_fp[u.pd].add(u.hash)
            if not (u.pkg in packages and u.mtime < bump):
                keep_art[u.pd].add(u.hash)

        # Refuse a layout this doesn't understand: every live library must have its
        # artifact in deps/ under the fingerprint's hash, or the hashes don't match up.
        libs = [u for u in live_units if u.kind.startswith("lib-") and u.pkg not in packages]
        present = set()
        for pd in pdirs:
            dd = os.path.join(pd, "deps")
            for f in os.listdir(dd) if os.path.isdir(dd) else ():
                m = HASH.search(f)
                if m:
                    present.add((pd, m.group(1)))
        missing = sum((u.pd, u.hash) not in present for u in libs)
        if missing > max(5, len(libs) // 50):
            return f"skipped: {missing} of {len(libs)} live libraries have no artifact under their hash"

        doomed = []   # (profile dir, path, is incremental)
        cutoff = max(bump, since)
        for pd in pdirs:
            for sub, keep in (("deps", keep_art[pd]), ("build", keep_art[pd]),
                              ("examples", keep_art[pd]), (".fingerprint", keep_fp[pd])):
                d = os.path.join(pd, sub)
                for f in os.listdir(d) if os.path.isdir(d) else ():
                    m = HASH.search(f)
                    if m and m.group(1) not in keep:
                        doomed.append((pd, os.path.join(d, f), False))
            d = os.path.join(pd, "incremental")
            for f in os.listdir(d) if os.path.isdir(d) else ():
                if os.path.getmtime(os.path.join(d, f)) < cutoff:
                    doomed.append((pd, os.path.join(d, f), True))

        # A file's space comes back only when its last link goes: an uplifted binary
        # keeps its deps/ twin, and incremental sessions hard-link each other's files.
        inodes = {}
        for pd, p, is_inc in doomed:
            for f in files_under(p):
                st = os.lstat(f)
                # Windows has no st_blocks
                n = st.st_blocks * 512 if hasattr(st, "st_blocks") else st.st_size
                e = inodes.setdefault((st.st_dev, st.st_ino) if st.st_ino else f,
                                      [n, st.st_nlink, 0, pd, is_inc])
                e[2] += 1
        freed = {pd: [0, 0] for pd in pdirs}
        for blocks, nlink, seen, pd, is_inc in inodes.values():
            if seen >= nlink:
                freed[pd][is_inc] += blocks
        failed = 0
        for _, p, _ in doomed:
            if listing:
                print(p)
            if not dry_run:
                try:
                    remove(p)
                except OSError:  # on Windows, a file something has open
                    failed += 1
        return freed, failed
    finally:
        unlock(held)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("target", nargs="?", default="target")
    ap.add_argument("--days", type=float, default=14,
                    help="a configuration not built for this long is dropped (default 14)")
    ap.add_argument("--dry-run", action="store_true", help="report, delete nothing")
    ap.add_argument("--list", action="store_true", help="print every path pruned")
    a = ap.parse_args()
    if not os.path.isdir(a.target):
        sys.exit(f"{a.target}: no such directory")
    packages, bump = workspace_packages(), version_bump_time()
    since = time.time() - a.days * 86400
    G = 1 << 30
    total = 0
    for tdir in target_dirs(os.path.normpath(a.target)):
        r = prune(tdir, packages, bump, since, a.dry_run, a.list)
        if isinstance(r, str):
            print(f"{tdir}: {r}")
            continue
        r, failed = r
        if failed:
            print(f"{tdir}: {failed} paths could not be deleted")
        for pd, (art, inc) in r.items():
            if art + inc >= G / 20:
                print(f"{pd}: {art / G:.1f} GB artifacts, {inc / G:.1f} GB incremental")
            total += art + inc
    print(f"{'Would free' if a.dry_run else 'Freed'} {total / G:.1f} GB")


if __name__ == "__main__":
    main()
