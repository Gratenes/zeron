#!/usr/bin/env python3
"""Is every fork feature still on this branch, and still wired in?

The fork (Gratenes/zeron, branch pc-combined) carries features on top of
upstream main. A merge of main can drop or break one without a conflict (a file
deleted upstream, a call site rewritten, a pin brought back). This checks the
registry (scripts/fork-features/registry.toml) against the working tree.

  check.py                 markers + the zgx pin + the registered tests' files (seconds)
  check.py --gate          before any build: fetch, 0 commits behind main, then the above
  check.py --risk          before a merge: which features touch files origin/main changed,
                           and which would conflict (git merge-tree, nothing is touched)
  check.py --survival      the registered commits' lines still in the tree, against
                           survival.json's floors
  check.py --bless         record today's survival as the floors (after a decision)
  check.py --tests         also run the features' Luau tests (zgx test ui <filter>)
  check.py --rust          also run the features' cargo tests (compiles: minutes)
  check.py --md            rewrite FORK_FEATURES.md from the registry
  check.py --only a,b      just those feature ids

Exit 0: nothing failed. Features with status "lost" are reported, not failing,
until restored (their markers then pass: it says to flip the status).
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent


def sh(*cmd: str, cwd: Path | None = None, check: bool = False) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, cwd=cwd or ROOT, capture_output=True, text=True, errors="replace", check=check)


def git(*args: str) -> subprocess.CompletedProcess:
    return sh("git", *args)


ROOT = Path(subprocess.run(["git", "-C", str(HERE), "rev-parse", "--show-toplevel"], capture_output=True, text=True).stdout.strip())
REG = tomllib.loads((HERE / "registry.toml").read_text())
META = REG["meta"]
FEATURES = REG["feature"]
FLOORS_PATH = HERE / "survival.json"
# What changes with every merge or bless and says nothing about a feature.
SURVIVAL_SKIP = re.compile(r"(\.base\.json|Cargo\.lock|Cargo\.toml|package-lock\.json|\.lock)$")

failures: list[str] = []
notes: list[str] = []


def fail(msg: str) -> None:
    failures.append(msg)
    print(f"  FAIL  {msg}")


def ok(msg: str) -> None:
    print(f"  ok    {msg}")


# --------------------------------------------------------------------- gate

def gate(fetch: bool) -> None:
    base = META["base"]
    print(f"gate: {base}")
    if fetch:
        r = git("fetch", "origin", "--quiet")
        if r.returncode:
            fail(f"git fetch origin failed: {r.stderr.strip()}")
    behind = git("rev-list", "--count", f"HEAD..{base}").stdout.strip()
    if behind != "0":
        fail(f"HEAD is {behind} commits behind {base}: merge it first (scripts/fork-features/README in FORK_FEATURES.md), don't build")
    else:
        ok(f"HEAD contains {base}")
    if git("status", "--porcelain", "--untracked-files=no").stdout.strip():
        notes.append("the working tree has uncommitted changes: a build from a SHA won't have them")


# ------------------------------------------------------------------ markers

def marker_ok(path: str, pattern: str) -> bool:
    if not git("ls-files", "--", path).stdout.strip():
        return False
    return git("grep", "-q", "-E", "-e", pattern, "--", path).returncode == 0


def check_markers(f: dict) -> bool:
    good = True
    for path, pattern in f.get("markers", []):
        if not marker_ok(path, pattern):
            good = False
            fail(f"{f['id']}: {path} has no /{pattern}/")
    return good


def check_pin() -> None:
    text = (ROOT / "apps/zgx/Cargo.toml").read_text()
    revs = set(re.findall(r'rev = "([0-9a-f]{40})"', text))
    pins = set(re.findall(r'^zgx(?:-[a-z]+)? = \{ git = "[^"]*", rev = "([0-9a-f]{40})"', text, re.M))
    want = META["zgx_pin"]
    print("zgx pin")
    if pins != {want}:
        fail(f"apps/zgx/Cargo.toml pins {sorted(pins) or sorted(revs)}, the registry says {want[:8]}: show the new pin carries "
             f"{', '.join(META['zgx_pin_carries'])} (git merge-base --is-ancestor in the zgx repo), then update zgx_pin in registry.toml")
        return
    ok(f"every zgx crate pins {want[:8]}")
    # Best effort: cargo's own clone of zgx, if this machine has built the pin.
    for db in sorted(Path.home().glob(".cargo/git/db/zgx-*")):
        if sh("git", "-C", str(db), "cat-file", "-e", want + "^{commit}").returncode:
            continue
        for c in META["zgx_pin_carries"]:
            r = sh("git", "-C", str(db), "merge-base", "--is-ancestor", c, want)
            if r.returncode:
                fail(f"zgx pin {want[:8]} does not carry {c}")
        ok("the pin carries " + ", ".join(META["zgx_pin_carries"]))
        return
    notes.append("zgx pin ancestry not verified here (no cargo clone has it)")


# ----------------------------------------------------------------- survival

def lines_of(ref_path: str) -> set[str] | None:
    r = git("show", ref_path)
    return None if r.returncode else {l.strip() for l in r.stdout.splitlines()}


def survival(f: dict) -> tuple[int, int, list[str]]:
    kept = total = 0
    missing: list[str] = []
    cache: dict[str, set[str] | None] = {}
    for c in f.get("commits", []):
        if git("cat-file", "-e", c + "^{commit}").returncode:
            missing.append(f"commit {c} is not in this history")
            continue
        diff = git("show", "--format=", "-U0", "--no-renames", c).stdout
        path = None
        for l in diff.splitlines():
            if l.startswith("+++ b/"):
                path = l[6:]
            elif l.startswith("+") and not l.startswith("+++") and path and not SURVIVAL_SKIP.search(path):
                t = l[1:].strip()
                if len(t) < 14 or re.match(r"^[\W_]*$|^(//|#|\*|/\*|--)", t):
                    continue
                if path not in cache:
                    cache[path] = lines_of(f"HEAD:{path}")
                total += 1
                if cache[path] is not None and t in cache[path]:
                    kept += 1
    return kept, total, missing


def check_survival(features: list[dict], bless: bool) -> None:
    floors = json.loads(FLOORS_PATH.read_text()) if FLOORS_PATH.exists() else {}
    new: dict[str, float] = dict(floors)
    print("survival (registered commits' added lines still in the tree)")
    for f in features:
        if not f.get("commits") or f.get("survival") is False:
            continue
        kept, total, missing = survival(f)
        for m in missing:
            notes.append(f"{f['id']}: {m}")
        if total == 0:
            continue
        ratio = kept / total
        floor = floors.get(f["id"])
        line = f"{f['id']}: {kept}/{total} = {ratio:.2f}" + (f" (floor {floor:.2f})" if floor is not None else " (no floor: --bless)")
        if bless:
            new[f["id"]] = round(max(0.0, ratio - 0.03), 2)
            ok(line + f" -> floor {new[f['id']]:.2f}")
        elif floor is not None and ratio < floor:
            if f.get("status") == "lost":
                print(f"  lost  {line}")
            else:
                fail(line + ": lines of this feature's commits are gone since the floor was set")
        else:
            ok(line)
    if bless:
        FLOORS_PATH.write_text(json.dumps(dict(sorted(new.items())), indent=1) + "\n")
        print(f"wrote {FLOORS_PATH.relative_to(ROOT)}")


# --------------------------------------------------------------------- risk

def feature_files(f: dict) -> set[str]:
    files: set[str] = set()
    for c in f.get("commits", []):
        files |= {p for p in git("show", "--format=", "--name-only", "--no-renames", c).stdout.splitlines() if p}
    for path, _ in f.get("markers", []):
        files.add(path)
    return files


def risk(features: list[dict]) -> None:
    base = META["base"]
    mb = git("merge-base", "HEAD", base).stdout.strip()
    changed = set(git("diff", "--name-only", "--no-renames", mb, base).stdout.splitlines())
    deleted = set(git("diff", "--name-only", "--diff-filter=D", mb, base).stdout.splitlines())
    ahead = git("rev-list", "--count", f"HEAD..{base}").stdout.strip()
    print(f"risk: {ahead} commits of {base} not merged yet, {len(changed)} files changed upstream")
    mt = git("merge-tree", "--write-tree", "--name-only", "--no-messages", "HEAD", base)
    conflicted: set[str] = set()
    if mt.returncode == 1:
        conflicted = {l for l in mt.stdout.splitlines()[1:] if l and "/" in l or (l and "." in l)}
        print(f"  the merge conflicts in {len(conflicted)} files")
    elif mt.returncode == 0:
        print("  the merge is clean (no textual conflicts: that says nothing about behaviour)")
    for f in features:
        touched = feature_files(f)
        hit = sorted(touched & changed)
        gone = sorted(touched & deleted)
        conf = sorted(touched & conflicted)
        if not hit:
            continue
        flag = "DELETED upstream" if gone else ("CONFLICTS" if conf else "touched")
        print(f"  {flag:16} {f['id']}: " + ", ".join((gone or conf or hit)[:4]) + (" …" if len(gone or conf or hit) > 4 else ""))
    # Each feature's own branch onto the new main: conflicts surface per feature.
    print("per-branch (each feature branch merged on its own onto the new main):")
    for f in features:
        br = f.get("branch")
        if not br:
            continue
        ref = next((r for r in (br, f"fork/{br}") if git("rev-parse", "--verify", "--quiet", r).returncode == 0), None)
        if not ref:
            print(f"  ?  {f['id']}: branch {br} not found locally (git fetch fork)")
            continue
        r = git("merge-tree", "--write-tree", "--name-only", "--no-messages", base, ref)
        if r.returncode == 0:
            print(f"  clean      {f['id']} ({ref})")
        elif r.returncode == 1:
            names = [l for l in r.stdout.splitlines()[1:] if l]
            print(f"  CONFLICTS  {f['id']} ({ref}): " + ", ".join(names[:4]))
        else:
            print(f"  ?          {f['id']} ({ref}): {r.stderr.strip()[:80]}")


# ------------------------------------------------------------------- tests

def zgx_cli() -> str | None:
    for c in (os.environ.get("ZGX"), shutil.which("zgx"), str(Path.home() / "zgx-cli/bin/zgx")):
        if c and Path(c).exists():
            return c
    return None


def run_luau(features: list[dict]) -> None:
    zgx = zgx_cli()
    print("luau tests (zgx test ui <filter>)")
    if not zgx:
        fail("no zgx CLI (set ZGX, or cargo install it at the pinned rev as CI does: .github/workflows/zgx-tests.yml)")
        return
    for f in features:
        for flt in f.get("luau", []):
            out = ""
            for attempt in (1, 2):
                r = sh(zgx, "test", "ui", flt, cwd=ROOT / "apps/zgx")
                out = r.stdout + r.stderr
                # A busy machine makes a mounted shell run out of its time budget: once more.
                if "time budget" not in out:
                    break
            m = re.search(r"(\d+) passed, (\d+) failed", out)
            if not m or int(m.group(1)) == 0:
                fail(f"{f['id']}: no test matches '{flt}' (renamed or deleted?)")
            elif int(m.group(2)):
                fail(f"{f['id']}: '{flt}': {m.group(2)} failed")
            else:
                ok(f"{f['id']}: '{flt}' ({m.group(1)})")


def run_rust(features: list[dict]) -> None:
    print("cargo tests")
    for f in features:
        for pkg, flt in f.get("rust", []):
            if pkg == "zeron-zgx":
                cmd, cwd = ["cargo", "test", "--locked", "--bin", "zeron-zgx", flt], ROOT / "apps/zgx"
            else:
                cmd, cwd = ["cargo", "test", "--locked", "-p", pkg, "--lib", flt], ROOT
            r = sh(*cmd, cwd=cwd)
            out = r.stdout + r.stderr
            m = re.search(r"test result: (\w+)\. (\d+) passed; (\d+) failed", out)
            if r.returncode or not m or int(m.group(2)) == 0:
                fail(f"{f['id']}: {pkg} '{flt}': " + ("no such test" if m and int(m.group(2)) == 0 else "failed"))
            else:
                ok(f"{f['id']}: {pkg} '{flt}' ({m.group(2)})")


def tests_exist(f: dict) -> None:
    """The feature's test files stay (a merge that deletes a test deletes the guard too)."""
    for path, _ in f.get("markers", []):
        if path.endswith((".test.luau", "_tests.rs")) and not git("ls-files", "--", path).stdout.strip():
            fail(f"{f['id']}: its test file {path} is gone")


# ---------------------------------------------------------------------- md

def write_md() -> None:
    rows = []
    for f in FEATURES:
        st = f.get("status", "present")
        tests = [f"luau `{t}`" for t in f.get("luau", [])] + [f"cargo `{p} {t}`" for p, t in f.get("rust", [])]
        rows.append(
            f"| `{f['id']}` | {f['title']}{(' **(' + st.upper() + ': ' + f['why'] + ')**') if st != 'present' and f.get('why') else ''} "
            f"| {f.get('owner', '')} | {('`' + f['branch'] + '`') if f.get('branch') else ''} "
            f"| {', '.join('`' + c + '`' for c in f.get('commits', [])[:4])}{' …' if len(f.get('commits', [])) > 4 else ''} "
            f"| {'<br>'.join(tests) or 'markers only'} |"
        )
    text = (HERE / "FORK_FEATURES.head.md").read_text().rstrip() + "\n\n"
    text += "| id | feature | owner chat | branch | commits | tests |\n|---|---|---|---|---|---|\n" + "\n".join(rows) + "\n"
    out = ROOT / "FORK_FEATURES.md"
    out.write_text(text)
    print(f"wrote {out.relative_to(ROOT)} ({len(FEATURES)} features)")


# -------------------------------------------------------------------- main

def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--gate", action="store_true")
    ap.add_argument("--no-fetch", action="store_true")
    ap.add_argument("--risk", action="store_true")
    ap.add_argument("--survival", action="store_true")
    ap.add_argument("--bless", action="store_true")
    ap.add_argument("--tests", action="store_true")
    ap.add_argument("--rust", action="store_true")
    ap.add_argument("--md", action="store_true")
    ap.add_argument("--only")
    a = ap.parse_args()

    if a.md:
        write_md()
        return 0
    feats = [f for f in FEATURES if not a.only or f["id"] in a.only.split(",")]
    if a.risk:
        risk(feats)
        return 0
    if a.bless:
        check_survival(feats, bless=True)
        return 0
    if a.gate:
        gate(fetch=not a.no_fetch)
    print("markers")
    restored, lost = [], []
    for f in feats:
        before = len(failures)
        good = check_markers(f)
        tests_exist(f)
        st = f.get("status", "present")
        if st == "lost":
            # known gone: not a failure; undo what it just added
            del failures[before:]
            (restored if good and f.get("markers") else lost).append(f)
            continue
        if good and len(failures) == before:
            ok(f"{f['id']}" + ("  (superseded)" if st == "superseded" else ""))
    check_pin()
    if a.survival:
        check_survival(feats, bless=False)
    if a.tests:
        run_luau([f for f in feats if f.get("status", "present") != "lost"])
    if a.rust:
        run_rust([f for f in feats if f.get("status", "present") != "lost"])
    print()
    for f in lost:
        print(f"LOST (restoration pending) {f['id']}: {f.get('why', '')} owner: {f.get('owner', '?')}")
    for f in restored:
        print(f"RESTORED? {f['id']}: its markers pass again: set status = \"present\" in registry.toml")
    for n in notes:
        print(f"note: {n}")
    if failures:
        print(f"\n{len(failures)} failed")
        return 1
    print("fork features: all present" + (f" ({len(lost)} known lost)" if lost else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
