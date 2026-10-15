import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts/release" / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


metadata = load("metadata", "metadata.py")
checksums = load("checksums", "checksums.py")
assets = load("assets", "verify-assets.py")


class ReleaseTests(unittest.TestCase):
    def test_only_main_pushes_and_matching_version_tags_publish(self):
        for event, kind, ref, publish, prerelease in [
            ("push", "branch", "main", "true", "true"),
            ("push", "branch", "feat/test", "false", "true"),
            ("pull_request", "branch", "main", "false", "true"),
            ("workflow_dispatch", "tag", f"v{metadata.version()}", "false", "true"),
            ("push", "tag", f"v{metadata.version()}", "true", "false"),
        ]:
            with self.subTest(event=event, kind=kind, ref=ref):
                result = metadata.resolve(event, kind, ref, "10", "2")
                self.assertEqual(result["publish"], publish)
                self.assertEqual(result["prerelease"], prerelease)
        with self.assertRaises(ValueError):
            metadata.resolve("push", "tag", "v0.0.0", "10", "1")

    def test_reruns_have_unique_prerelease_tags(self):
        self.assertNotEqual(metadata.resolve("push", "branch", "main", "10", "1")["release_tag"],
                            metadata.resolve("push", "branch", "main", "10", "2")["release_tag"])

    def test_checksums_reject_tampering_and_path_escape(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            (directory / "app.zip").write_bytes(b"original")
            checksums.write(directory, "SHA256SUMS")
            checksums.check(directory, "SHA256SUMS")
            (directory / "app.zip").write_bytes(b"tampered")
            with self.assertRaises(ValueError):
                checksums.check(directory, "SHA256SUMS")
            (directory / "SHA256SUMS").write_text("bad  ../outside\n")
            with self.assertRaises(ValueError):
                checksums.check(directory, "SHA256SUMS")

    def test_release_requires_both_platforms_and_real_pe_executable(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            for name in ("TTREE_1.0.0_universal.dmg", "TTREE_universal.app.tar.gz",
                         "TTREE_universal.app.tar.gz.sig", "latest.json"):
                (directory / name).write_bytes(b"fixture")
            with self.assertRaises(ValueError):
                assets.verify(directory, "1.0.0")
            zip_path = directory / "TTREE_1.0.0_windows_x64.zip"
            with zipfile.ZipFile(zip_path, "w") as archive:
                archive.writestr("TTREE/TTREE.exe", b"MZfixture")
            assets.verify(directory, "1.0.0")
            with zipfile.ZipFile(zip_path, "w") as archive:
                archive.writestr("TTREE/TTREE.exe", b"not an executable")
            with self.assertRaises(ValueError):
                assets.verify(directory, "1.0.0")

    def test_update_signature_is_required_before_both_publication_checks(self):
        workflow = (ROOT / ".github/workflows/build.yml").read_text()
        self.assertLess(workflow.index("python3 scripts/release/checksums.py dist/release --write SHA256SUMS"),
                        workflow.index("run: scripts/release/sign-checksums.sh dist/release"))
        publish = (ROOT / "scripts/release/publish.sh").read_text()
        for directory in ("$RELEASE_DIR", "$DOWNLOADED"):
            self.assertIn(f'scripts/release/verify-checksums-signature.sh "{directory}"', publish)
        self.assertLess(publish.index('verify-checksums-signature.sh "$DOWNLOADED"'),
                        publish.index('--draft=false'))
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            (directory / "app.zip").write_bytes(b"original")
            (directory / "SHA256SUMS.sig").write_text("stale signature")
            checksums.write(directory, "SHA256SUMS")
            self.assertNotIn("SHA256SUMS.sig", (directory / "SHA256SUMS").read_text())
            (directory / "SHA256SUMS.sig").write_text("new signature")
            checksums.check(directory, "SHA256SUMS")

    def test_workspace_and_workflow_use_current_source(self):
        self.assertTrue((ROOT / "apps/macos/src/main.rs").exists())
        self.assertTrue((ROOT / "apps/windows/src/main.rs").exists())
        self.assertFalse((ROOT / "crates/ttree-app").exists())
        workflow = (ROOT / ".github/workflows/build.yml").read_text()
        self.assertIn("needs: [metadata, macos, windows]", workflow)
        self.assertIn("scripts/release/publish.sh", workflow)
        self.assertNotIn("npm run", workflow)
        self.assertNotIn("src-tauri/", workflow)
        for name in ("AGENTS.md", "README.md", "CLAUDE.md"):
            text = (ROOT / name).read_text()
            self.assertNotIn("npm run tauri", text)
            self.assertNotIn("src-tauri/", text)


if __name__ == "__main__":
    unittest.main()
