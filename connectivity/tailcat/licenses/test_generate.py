"""Portable license-byte regression tests (no network or Go build required)."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("licenses_generate", ROOT / "generate.py")
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


class PortableBundleTests(unittest.TestCase):
    def test_generated_metadata_uses_utf8_lf_on_windows(self):
        original = Path.write_text

        def windows_write(path, data, *args, **kwargs):
            kwargs.setdefault("newline", "\r\n")
            kwargs.setdefault("encoding", "cp1252")
            return original(path, data, *args, **kwargs)

        with tempfile.TemporaryDirectory(dir=ROOT) as directory:
            root = Path(directory)
            (root / "go.mod").write_bytes(b"go 1.27.1\n")
            for name in ("LICENSE", "PATENTS"):
                (root / name).write_bytes(b"Exact upstream bytes\r\n")
            with patch.object(generator, "dependency_modules", return_value={}), \
                 patch.object(generator, "MODULE_DIR", root), \
                 patch.object(generator, "run", side_effect=[str(root), "go1.27.1"]), \
                 patch.object(Path, "write_text", windows_write):
                generator.generate("go", root / "bundle")
            for name in ("README.md", "DEPENDENCIES.txt"):
                self.assertNotIn(b"\r", (root / "bundle" / name).read_bytes())
            self.assertEqual((root / "bundle/go/LICENSE.txt").read_bytes(), b"Exact upstream bytes\r\n")

    def test_git_autocrlf_preserves_license_bytes(self):
        with tempfile.TemporaryDirectory(dir=ROOT) as directory:
            root = Path(directory)
            def git(*args):
                subprocess.run(["git", "-C", str(root), *args], check=True,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            git("init")
            git("config", "core.autocrlf", "true")
            (root / ".gitattributes").write_bytes((ROOT / ".gitattributes").read_bytes())
            (root / "bundle").mkdir()
            data = b"Copyright UTF-8: \xe2\x80\x9d\nExact bytes\n"
            target = root / "bundle/LICENSE.txt"
            target.write_bytes(data)
            git("add", ".")
            target.unlink()
            git("checkout-index", "--all")
            self.assertEqual(target.read_bytes(), data)


if __name__ == "__main__":
    unittest.main()
