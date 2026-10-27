#!/usr/bin/env python3
"""Delete old GitHub releases, keeping each finished series' first and last version.

A series is a major.minor (1.5, 1.6, ...). The current series (the highest one, or the
release being made's, whichever is higher) keeps every release. Every older series keeps
its first and last version and loses the rest: v1.5.0 ... v1.5.43 keeps v1.5.0 and
v1.5.43. Versions order as semver: v1.0.0-alpha < v1.0.0-beta < v1.0.0 < v1.0.1.

Drafts and tags that aren't vMAJOR.MINOR.PATCH[-PRE] are never touched, and neither is the
release named by --current. Only the releases go: their git tags stay.

Usage (from the repo, gh logged in):
  prune-releases.py --current v1.7.24 --dry-run   # print the plan only
  prune-releases.py --current v1.7.24             # delete
Exits 1 if --current isn't a published release (so nothing is deleted after a failed
upload) or if any deletion failed.
"""
import argparse
import json
import re
import subprocess
import sys

TAG = re.compile(r'^v(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$')


def parse(tag):
    m = TAG.match(tag)
    if not m:
        return None
    major, minor, patch, pre = int(m[1]), int(m[2]), int(m[3]), m[4]
    # A pre-release sorts before its release; its identifiers compare as semver's do
    # (numbers numerically and before words).
    rank = (1,) if pre is None else (0,) + tuple(
        (0, int(p), '') if p.isdigit() else (1, 0, p) for p in pre.split('.'))
    return (major, minor), (major, minor, patch, rank)


def plan(tags, current):
    """The tags to delete, and each series' kept tags, from the published tags."""
    series = {}
    for tag in tags:
        p = parse(tag)
        if p:
            series.setdefault(p[0], []).append((p[1], tag))
    if not series:
        return [], {}
    newest = max(series)
    cur = parse(current) if current else None
    if cur and cur[0] > newest:
        newest = cur[0]
    delete, kept = [], {}
    for key in sorted(series):
        versions = [t for _, t in sorted(series[key])]
        if key == newest or len(versions) <= 2:
            keep = versions
        else:
            keep = [versions[0], versions[-1]]
        keep = keep + [t for t in versions if t == current and t not in keep]
        kept[key] = keep
        delete += [t for t in versions if t not in keep]
    return delete, kept


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--current', help='the release this run made (never deleted)')
    ap.add_argument('--dry-run', action='store_true')
    args = ap.parse_args()
    out = subprocess.run(['gh', 'release', 'list', '--limit', '1000', '--json', 'tagName,isDraft'],
                         check=True, capture_output=True, text=True).stdout
    published = [r['tagName'] for r in json.loads(out) if not r['isDraft']]
    if args.current and args.current not in published:
        sys.exit(f'{args.current} is not a published release: deleting nothing')
    delete, kept = plan(published, args.current)
    for (major, minor), tags in kept.items():
        print(f'keep {major}.{minor}: {" ".join(tags)}')
    if not delete:
        print('Nothing to delete.')
        return
    print(f'{"Would delete" if args.dry_run else "Deleting"} {len(delete)}: {" ".join(delete)}')
    if args.dry_run:
        return
    failed = []
    for tag in delete:
        r = subprocess.run(['gh', 'release', 'delete', tag, '--yes'], capture_output=True, text=True)
        if r.returncode != 0:
            failed.append(tag)
            print(f'FAILED {tag}: {r.stderr.strip()}')
    print(f'Deleted {len(delete) - len(failed)} of {len(delete)}.')
    if failed:
        sys.exit(1)


if __name__ == '__main__':
    main()
