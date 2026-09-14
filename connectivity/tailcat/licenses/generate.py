#!/usr/bin/env python3
"""Generate redistribution licenses for the statically linked Tailcat adapter.

The dependency graph and source versions come from the checked-in go.mod/go.sum.
License bytes are copied from the exact modules in Go's verified module cache;
no license text is fetched from repository URLs.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

SCRIPT_DIR = Path(__file__).resolve().parent
MODULE_DIR = SCRIPT_DIR.parent
BUNDLE_DIR = SCRIPT_DIR / "bundle"
PACKAGE = "./cmd/kratos-tailcat"
TARGETS = (
    ("linux", "amd64"),
    ("linux", "arm64"),
    ("darwin", "amd64"),
    ("darwin", "arm64"),
    ("windows", "amd64"),
    ("windows", "arm64"),
)
LICENSE_NAMES = (
    "LICENSE",
    "LICENSE.txt",
    "LICENSE.md",
    "COPYING",
    "COPYING.txt",
    "COPYING.md",
    "NOTICE",
    "NOTICE.txt",
    "NOTICE.md",
    "PATENTS",
)


def run(go: str, *args: str, env: dict[str, str] | None = None) -> str:
    return subprocess.run(
        (go, *args),
        cwd=MODULE_DIR,
        env=env,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
    ).stdout


def decode_stream(value: str) -> list[dict]:
    decoder = json.JSONDecoder()
    items = []
    offset = 0
    while offset < len(value):
        while offset < len(value) and value[offset].isspace():
            offset += 1
        if offset == len(value):
            break
        item, offset = decoder.raw_decode(value, offset)
        items.append(item)
    return items


def dependency_modules(go: str) -> dict[str, tuple[str, Path]]:
    modules: dict[str, tuple[str, Path]] = {}
    for goos, goarch in TARGETS:
        env = os.environ.copy()
        env.update({"CGO_ENABLED": "0", "GOOS": goos, "GOARCH": goarch})
        packages = decode_stream(run(go, "list", "-deps", "-json", PACKAGE, env=env))
        for package in packages:
            module = package.get("Module")
            if not module or module.get("Main"):
                continue
            module = module.get("Replace") or module
            path = module.get("Path", "")
            version = module.get("Version", "")
            directory = module.get("Dir", "")
            if not path or not version or not directory:
                raise RuntimeError(f"dependency has incomplete module metadata: {module!r}")
            value = (version, Path(directory))
            if path in modules and modules[path] != value:
                raise RuntimeError(f"multiple versions selected for {path}")
            modules[path] = value
    return modules


def source_files(root: Path) -> list[Path]:
    files = [root / name for name in LICENSE_NAMES if (root / name).is_file()]
    if not files:
        raise RuntimeError(f"no top-level license file found in module cache directory {root}")
    for path in files:
        if path.is_symlink():
            raise RuntimeError(f"refusing symlinked license file {path}")
    return files


def safe_stem(index: int, module: str, version: str) -> str:
    leaf = module.rsplit("/", 1)[-1]
    leaf = re.sub(r"[^A-Za-z0-9._-]", "_", leaf).strip("._-") or "module"
    digest = hashlib.sha256(f"{module}@{version}".encode()).hexdigest()[:12]
    return f"{index:03d}-{leaf}-{digest}"


def file_map(root: Path) -> dict[str, tuple[bytes, int]]:
    if not root.exists():
        return {}
    return {
        path.relative_to(root).as_posix(): (path.read_bytes(), path.stat().st_mode & 0o777)
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


def generate(go: str, destination: Path) -> None:
    modules = dependency_modules(go)
    destination.mkdir(parents=True)
    manifest = ["module\tversion\tlicense_files"]
    for index, (module, (version, root)) in enumerate(sorted(modules.items()), 1):
        stem = safe_stem(index, module, version)
        outputs = []
        for source in source_files(root):
            suffix = re.sub(r"[^A-Za-z0-9_-]", "_", source.name)
            output = f"modules/{stem}-{suffix}.txt"
            target = destination / output
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.read_bytes())

            target.chmod(0o644)
            outputs.append(output)
        manifest.append(f"{module}\t{version}\t{','.join(outputs)}")

    goroot = Path(run(go, "env", "GOROOT").strip())
    goversion = run(go, "env", "GOVERSION").strip()
    required = re.search(r"^go (\S+)$", (MODULE_DIR / "go.mod").read_text(), re.MULTILINE)
    if not required or goversion != f"go{required.group(1)}":
        raise RuntimeError(f"generator requires Go {required.group(1) if required else '?'}; found {goversion}")
    go_outputs = []
    for name in ("LICENSE", "PATENTS"):
        source = goroot / name
        if not source.is_file() or source.is_symlink():
            raise RuntimeError(f"missing regular Go runtime {name}: {source}")
        output = f"go/{name}.txt"
        target = destination / output
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(source.read_bytes())

        target.chmod(0o644)
        go_outputs.append(output)
    manifest.append(f"Go standard library/runtime\t{goversion}\t{','.join(go_outputs)}")
    dependencies = destination / "DEPENDENCIES.txt"
    dependencies.write_text("\n".join(manifest) + "\n", encoding="utf-8", newline="\n")
    dependencies.chmod(0o644)
    targets = ", ".join(f"{goos}/{goarch}" for goos, goarch in TARGETS)
    readme = destination / "README.md"
    readme.write_text(
        "# Tailcat adapter redistribution licenses\n\n"
        "This directory contains the license and patent-notice files for the Go modules "
        "compiled into the statically linked `kratos-tailcat` adapter, plus the Go standard "
        f"library/runtime license. Source versions are recorded in `DEPENDENCIES.txt`.\n\n"
        f"Generated for `{PACKAGE}` on {targets} with {goversion}. The generator resolves the "
        "dependency closure from the checked-in `go.mod`/`go.sum` and copies bytes from the "
        "corresponding verified Go module-cache directories. It does not download license text "
        "from repository URLs.\n",
        encoding="utf-8",
        newline="\n",
    )
    readme.chmod(0o644)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="fail if the checked-in bundle differs")
    args = parser.parse_args()
    go = os.environ.get("GO") or shutil.which("go")
    if not go:
        raise SystemExit("go is required (or set GO to its path)")
    with tempfile.TemporaryDirectory(prefix="tailcat-licenses-", dir=SCRIPT_DIR) as temporary:
        generated = Path(temporary) / "bundle"
        generate(go, generated)
        if args.check:
            if file_map(generated) != file_map(BUNDLE_DIR):
                raise SystemExit("Tailcat license bundle is stale; run connectivity/tailcat/licenses/generate.py")
            return
        if file_map(generated) == file_map(BUNDLE_DIR):
            return
        if BUNDLE_DIR.exists():
            shutil.rmtree(BUNDLE_DIR)
        generated.rename(BUNDLE_DIR)


if __name__ == "__main__":
    main()
