#!/usr/bin/env python3
"""Package metadata, contents and upgrade ordering checks (no install needed)."""
import importlib.util
from functools import cmp_to_key
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("packaging", Path(__file__).resolve().parents[1] / "scripts/package-linux.py")
packaging = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packaging)
REQUIRE = "--require-comparators" in sys.argv
INTEGRATION = "--integration" in sys.argv
sys.argv = [arg for arg in sys.argv if arg not in ("--require-comparators", "--integration")]


class PackageTests(unittest.TestCase):
    def test_versions(self):
        for kind in packaging.DEPENDENCIES:
            self.assertEqual(packaging.package_version("v1.2.3+build.7", kind), "1.2.3")
            prefix = "1.2.3pre" if kind == "pacman" else "1.2.3~"
            for prerelease in ("alpha.1", "rc.1", "01a", "alpha-1", "alpha.1", "-", "99999999999999999999"):
                with self.subTest(kind=kind, prerelease=prerelease):
                    version = packaging.package_version("v1.2.3-" + prerelease, kind)
                    self.assertTrue(version.startswith(prefix))
                    self.assertRegex(version[len(prefix):], r"^[a-z]+$")
                    self.assertEqual(version, packaging.package_version("v1.2.3-" + prerelease + "+build.007", kind))
            self.assertNotEqual(packaging.package_version("v1.2.3-alpha-1", kind),
                                packaging.package_version("v1.2.3-alpha.1", kind))

    def test_invalid_semver_is_rejected(self):
        for tag in ("../../bad-tag", "1.2", "1.2.3.4", "01.2.3", "1.02.3", "1.2.03",
                    "1.2.3-", "1.2.3-a..b", "1.2.3-.a", "1.2.3-a.", "1.2.3-01",
                    "1.2.3-alpha.00", "1.2.3+", "1.2.3+a..b", "1.2.3-a_b", "１.2.3"):
            for kind in packaging.DEPENDENCIES:
                with self.subTest(tag=tag, kind=kind), self.assertRaises(ValueError):
                    packaging.package_version(tag, kind)

    def test_upgrade_ordering(self):
        available = {"deb": shutil.which("dpkg"), "pacman": shutil.which("vercmp")}
        try:
            import rpm
            available["rpm"] = True
        except ImportError:
            available["rpm"] = False
        # Independent SemVer reference comparison, without the package encoding:
        # numeric IDs sort numerically before nonnumeric IDs, other IDs compare
        # by ASCII, and a shorter list wins once all common IDs are equal.
        def compare_prerelease(left, right):
            a, b = left.split("."), right.split(".")
            for x, y in zip(a, b):
                if x == y:
                    continue
                if x.isdigit() and y.isdigit():
                    return -1 if int(x) < int(y) else 1
                if x.isdigit() != y.isdigit():
                    return -1 if x.isdigit() else 1
                return -1 if x < y else 1
            return (len(a) > len(b)) - (len(a) < len(b))

        standard = ["alpha", "alpha.1", "alpha.beta", "beta", "beta.2", "beta.11", "rc.1"]
        identifiers = ["0", "1", "2", "9", "10", "11", "99", "100", "99999999999999999999",
                       "-", "--", "-0", "0-", "01a", "A", "Z", "a", "a-", "aa", "alpha-1",
                       "alpha.1", "alpha.1.0", "alpha.1.0.0", "alpha.1.1", "alpha.1.A", "alpha.1.a",
                       "alpha.1.aa", "alpha.a", "alpha.a.0", "alpha.a-", "alpha.a0", "alpha.aa"]
        # Every permitted ASCII character must retain its lexical order inside a
        # nonnumeric identifier, including case and digits adjacent to letters.
        identifiers += ["X" + c for c in "-0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"]
        cases = sorted(set(identifiers + standard), key=cmp_to_key(compare_prerelease))
        for kind in packaging.DEPENDENCIES:
            if not available[kind]:
                if REQUIRE:
                    self.fail(f"Required {kind} comparator unavailable")
                print(f"SKIP {kind} comparator (unavailable)")
                continue
            versions = [packaging.package_version("1.0.0-" + pre, kind) for pre in cases]
            self.assertEqual(len(set(versions)), len(cases), "distinct SemVer prereleases must not collide")
            versions += [packaging.package_version(tag, kind) for tag in
                         ("1.0.0", "1.0.1-0", "1.0.1", "1.1.0-0", "2.0.0-0")]
            pairs = [(f"1:{a}-1", f"1:{b}-1") for a, b in zip(versions, versions[1:])]
            # Standard's published ordering is checked directly as well as via
            # the reference comparator and larger boundary-case set.
            standard_versions = [packaging.package_version("1.0.0-" + pre, kind) for pre in standard]
            pairs += [(f"1:{a}-1", f"1:{b}-1") for a, b in zip(standard_versions, standard_versions[1:])]
            # The epoch also fixes upgrades from the previously published form.
            legacy = "0.1.0-alpha" if kind == "deb" else "0.1.0.alpha-1"
            pairs.append((legacy, "1:0.1.0-1"))
            for older, newer in pairs:
                with self.subTest(kind=kind, older=older, newer=newer):
                    if kind == "deb":
                        self.assertEqual(subprocess.run(["dpkg", "--compare-versions", older, "lt", newer]).returncode, 0)
                    elif kind == "pacman":
                        self.assertLess(int(subprocess.check_output(["vercmp", older, newer])), 0)
                    else:
                        def evr(value):
                            epoch, sep, rest = value.partition(":")
                            if not sep:
                                epoch, rest = "0", epoch
                            version, sep, release = rest.rpartition("-")
                            return (epoch, version, release) if sep else (epoch, rest, "")
                        self.assertLess(rpm.labelCompare(evr(older), evr(newer)), 0)
            print(f"{kind}: {len(pairs)} real version comparisons passed")

    def test_all_package_payloads_and_dependencies(self):
        tag = "v0.1.0-alpha-1+build.007"
        with tempfile.TemporaryDirectory(prefix="package-test-") as tmp:
            root = Path(tmp)
            for directory in ("chrome", "firefox", "scripts", "models"):
                (root / directory).mkdir()
            for filename in ("README.md", "README.zh-CN.md", "LICENSE", "translate"):
                (root / filename).write_text("fixture\n")
            (root / "translate").chmod(0o755)
            for name in packaging.MODELS:
                (root / "models" / name).write_text(name)
            calls = []

            def fake_fpm(command, check):
                stage = Path(command[command.index("-C") + 1])
                kind = command[command.index("-t") + 1]
                name = command[command.index("-n") + 1]
                dependencies = [command[i + 1] for i, v in enumerate(command) if v == "--depends"]
                self.assertEqual(dependencies, list(packaging.DEPENDENCIES[kind]))
                self.assertEqual(command[command.index("--epoch") + 1], "1")
                self.assertEqual(command[command.index("-v") + 1], packaging.package_version(tag, kind))
                self.assertIn(tag, command[command.index("-p") + 1])
                self.assertIn(tag, command[command.index("--description") + 1])
                self.assertTrue((stage / "usr/bin/translate").is_file())
                for model in packaging.MODELS:
                    self.assertEqual((stage / "usr/share/qwen-translate" / model).exists(), name.endswith("-full"))
                calls.append((kind, name))

            with patch.object(packaging.subprocess, "run", side_effect=fake_fpm):
                packaging.build_packages(root, root / "translate", root / "models", root / "out", tag)
            self.assertEqual(len(set(calls)), 6)

            if not INTEGRATION:
                return
            packaging.build_packages(root, root / "translate", root / "models", root / "out", tag)
            artifacts = list((root / "out").iterdir())
            self.assertEqual(len(artifacts), 6)
            for artifact in artifacts:
                kind = "deb" if artifact.suffix == ".deb" else "rpm" if artifact.suffix == ".rpm" else "pacman"
                if kind == "deb":
                    metadata = subprocess.check_output(["dpkg-deb", "-f", str(artifact)], text=True)
                    contents = subprocess.check_output(["dpkg-deb", "-c", str(artifact)], text=True)
                elif kind == "rpm":
                    metadata = subprocess.check_output(["rpm", "-qp", "--qf", "%{EPOCH}:%{VERSION}\n%{DESCRIPTION}\n", str(artifact)], text=True)
                    metadata += subprocess.check_output(["rpm", "-qp", "--requires", str(artifact)], text=True)
                    contents = subprocess.check_output(["rpm", "-qpl", str(artifact)], text=True)
                else:
                    metadata = subprocess.check_output(["bsdtar", "-xOf", str(artifact), ".PKGINFO"], text=True)
                    contents = subprocess.check_output(["bsdtar", "-tf", str(artifact)], text=True)
                self.assertIn("1:" + packaging.package_version(tag, kind), metadata)
                self.assertIn(tag, metadata)
                self.assertIn(tag, artifact.name)
                for dependency in packaging.DEPENDENCIES[kind]:
                    self.assertIn(dependency, metadata)
                self.assertIn("usr/bin/translate", contents)
                for model in packaging.MODELS:
                    self.assertEqual(model in contents, artifact.name.startswith("qwen-translate-full"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
