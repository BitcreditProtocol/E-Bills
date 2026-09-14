#!/usr/bin/env python3
"""Exercise native npm packaging and simulated GitHub/npm failures without writes."""

import base64
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import release_wasm as release


class Terminated(RuntimeError):
    pass


class Remote:
    def __init__(self, folder, ctx):
        self.ctx = ctx
        self.folder = folder
        self.plan = json.loads((folder / release.PLAN).read_text())
        self.tag = None
        self.release = None
        self.assets = {}
        self.integrity = None
        self.writes = []
        self.stop = None
        self.lost = None
        self.artifact = {"id": 5, "name": release.ARTIFACT, "expired": False}
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as archive:
            for file in folder.rglob("*"):
                if file.is_file():
                    archive.writestr(file.relative_to(folder).as_posix(), file.read_bytes())
        self.archive = buffer.getvalue()

    def wrote(self, operation):
        self.writes.append(operation)
        if self.stop == operation:
            self.stop = None
            raise Terminated(operation)
        if self.lost == operation:
            self.lost = None
            raise release.ReleaseError("Response lost after " + operation)

    def gh(self, endpoint, *, method="GET", data=None, **kwargs):
        if "/artifacts/5/zip" in endpoint:
            return self.archive
        if "/artifacts?" in endpoint:
            return [{"total_count": 1 if self.artifact else 0, "artifacts": [self.artifact] if self.artifact else []}]
        if "/git/ref/tags/" in endpoint:
            return {"object": {"type": "commit", "sha": self.tag}} if self.tag else None
        if endpoint.endswith("/git/refs"):
            self.tag = data["sha"]
            self.wrote("tag")
            return {}
        if endpoint.endswith("/releases?per_page=100"):
            return [[copy.deepcopy(self.release)] if self.release else []]
        if endpoint.endswith("/releases") and method == "POST":
            self.release = {"id": 7, **data}
            self.wrote("release")
            return self.release
        if "/assets?" in endpoint:
            return [list(self.assets.values())]
        if endpoint.endswith("/releases/7") and method == "PATCH":
            self.release.update(data)
            self.wrote("final")
            return self.release
        raise AssertionError((endpoint, method))

    def command(self, args, **kwargs):
        if args[:3] == ["gh", "release", "upload"]:
            path = Path(args[4])
            self.assets[path.name] = {"id": len(self.assets) + 10, "name": path.name, "state": "uploaded",
                                     "size": path.stat().st_size, "digest": "sha256:" + release.digest(path).hex()}
            try:
                self.wrote("asset:" + path.name)
            except release.ReleaseError:
                return subprocess.CompletedProcess(args, 1, b"", b"lost")
            return subprocess.CompletedProcess(args, 0, b"", b"")
        if args[:2] == ["npm", "publish"]:
            self.integrity = self.plan["integrity"]
            try:
                self.wrote("npm")
            except release.ReleaseError:
                return subprocess.CompletedProcess(args, 1, b"", b"lost")
            return subprocess.CompletedProcess(args, 0, b"", b"")
        raise AssertionError("Unexpected process (real network disabled): " + str(args))


class ReleaseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.root = Path(cls.tmp.name)
        cls.source = cls.root / "pkg"
        cls.source.mkdir()
        cls.ctx = dict(repository="BitcreditProtocol/Bitcredit-Core", sha="a" * 40, run_id="123",
                       version="1.2.3", tag="v1.2.3", package_name="@bitcredit/bcr-ebill-wasm")
        (cls.source / "package.json").write_text(json.dumps({"name": cls.ctx["package_name"], "version": "1.2.3",
                                                           "files": ["index.js", "index.d.ts", "index_bg.wasm"]}))
        for name in ("index.js", "index.d.ts", "index_bg.wasm", "LICENSE"):
            (cls.source / name).write_text("fixture\n")
        cls.folder = cls.root / "saved"
        with patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": ""}):
            release.prepare(cls.folder, cls.source, cls.ctx)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def run_publish(self, remote):
        with patch.object(release, "gh", side_effect=remote.gh), \
                patch.object(release, "command", side_effect=remote.command), \
                patch.object(release, "npm_integrity", side_effect=lambda *args: remote.integrity), \
                patch.object(release, "canonical_version", return_value="1.2.3"), \
                patch.object(release, "note"):
            release.publish(self.folder, self.ctx)

    def test_success_and_complete_repeat_preserve_every_write(self):
        remote = Remote(self.folder, self.ctx)
        self.run_publish(remote)
        before = list(remote.writes)
        self.run_publish(remote)
        self.assertEqual(remote.writes, before)
        self.assertFalse(remote.release["draft"])
        self.assertEqual(remote.integrity, remote.plan["integrity"])

    def test_process_stop_after_each_write_recovers_original_bytes(self):
        assets = ["asset:" + Path(name).name for name in json.loads((self.folder / release.PLAN).read_text())["files"]
                  if name.startswith("assets/")]
        for operation in ["tag", "release", *assets, "npm", "final"]:
            with self.subTest(operation=operation):
                remote = Remote(self.folder, self.ctx)
                remote.stop = operation
                with self.assertRaises(Terminated):
                    self.run_publish(remote)
                self.run_publish(remote)
                self.assertFalse(remote.release["draft"])
                self.assertEqual(remote.writes.count(operation), 1)

    def test_lost_responses_are_read_back_before_continuing(self):
        for operation in ("tag", "release", "asset:LICENSE", "npm", "final"):
            with self.subTest(operation=operation):
                remote = Remote(self.folder, self.ctx)
                remote.lost = operation
                self.run_publish(remote)
                self.assertFalse(remote.release["draft"])
                self.assertEqual(remote.writes.count(operation), 1)

    def test_conflicting_tag_package_or_asset_stops_without_writes(self):
        for conflict in ("tag", "npm", "asset"):
            with self.subTest(conflict=conflict):
                remote = Remote(self.folder, self.ctx)
                if conflict == "tag":
                    remote.tag = "b" * 40
                elif conflict == "npm":
                    remote.integrity = "sha512-wrong"
                else:
                    remote.tag = self.ctx["sha"]
                    remote.release = {"id": 7, "tag_name": self.ctx["tag"], "draft": True}
                    remote.assets["LICENSE"] = {"id": 10, "name": "LICENSE", "state": "uploaded", "size": 8,
                                               "digest": "sha256:" + "b" * 64}
                with self.assertRaises(release.ReleaseError):
                    self.run_publish(remote)
                self.assertEqual(remote.writes, [])

    def test_missing_expired_or_corrupted_artifact_stops(self):
        for failure in ("missing", "expired", "corrupt"):
            with self.subTest(failure=failure):
                remote = Remote(self.folder, self.ctx)
                remote.tag = self.ctx["sha"]
                if failure == "missing":
                    remote.artifact = None
                elif failure == "expired":
                    remote.artifact["expired"] = True
                else:
                    remote.archive = b"not a zip"
                with self.assertRaises((release.ReleaseError, zipfile.BadZipFile)):
                    self.run_publish(remote)
                self.assertEqual(remote.writes, [])

    def test_incomplete_paginated_rows_stop_before_writes(self):
        for kind in ("release", "asset", "artifact", "empty-envelope"):
            with self.subTest(kind=kind):
                remote = Remote(self.folder, self.ctx)
                if kind == "asset":
                    remote.tag = self.ctx["sha"]
                    remote.release = {"id": 7, "tag_name": self.ctx["tag"], "draft": True}
                original = remote.gh
                def damaged(endpoint, **kwargs):
                    if kind == "release" and endpoint.endswith("/releases?per_page=100"):
                        return [[{"id": 7, "draft": True}]]
                    if kind == "asset" and "/assets?" in endpoint:
                        return [[{"id": 10, "size": 8, "state": "uploaded"}]]
                    if kind == "artifact" and "/artifacts?" in endpoint:
                        return [{"total_count": 1, "artifacts": [{"id": 5, "expired": False}]}]
                    if kind == "empty-envelope" and "/artifacts?" in endpoint:
                        return []
                    return original(endpoint, **kwargs)
                remote.gh = damaged
                with self.assertRaises(release.ReleaseError):
                    self.run_publish(remote)
                self.assertEqual(remote.writes, [])

    def test_restoration_requires_original_run_and_sha(self):
        remote = Remote(self.folder, self.ctx)
        for key, value in (("sha", "b" * 40), ("run_id", "124")):
            with self.subTest(key=key), tempfile.TemporaryDirectory() as tmp, \
                    patch.object(release, "gh", side_effect=remote.gh):
                with self.assertRaises(release.ReleaseError):
                    release.restore(Path(tmp), {**self.ctx, key: value})

    def restore_absent(self, artifact_pages, history=None, attempt="1"):
        def read(endpoint, **kwargs):
            if "/artifacts?" in endpoint:
                return artifact_pages
            previous = int(endpoint.split("/attempts/")[1].split("/")[0])
            value = history[previous]
            if isinstance(value, Exception):
                raise value
            return value
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, {"GITHUB_RUN_ATTEMPT": attempt}), \
                patch.object(release, "gh", side_effect=read), patch.object(release, "tag_sha", return_value=None), \
                patch.object(release, "release", return_value=None), patch.object(release, "npm_integrity", return_value=None), \
                patch.object(release, "canonical_version", return_value="1.2.3"):
            destination = Path(tmp) / "restored"
            result = release.restore(destination, self.ctx)
            self.assertFalse(destination.exists())
            return result

    def preparation_history(self, *, conclusion="skipped", attempt=1):
        return [{"total_count": 1, "jobs": [{"id": 10, "run_id": 123, "run_attempt": attempt,
            "head_sha": self.ctx["sha"], "name": "WASM Release and Publish", "status": "completed",
            "conclusion": "failure", "steps": [{"number": 12, "name": "Save the package before any publication writes",
                                                  "status": "completed", "conclusion": conclusion}]}]}]

    def test_artifact_counts_cannot_turn_incomplete_read_into_preparation(self):
        row = {"id": 6, "name": "other", "expired": False}
        for pages in ([{"total_count": 1, "artifacts": []}], [{"artifacts": []}],
                      [{"total_count": True, "artifacts": [row]}],
                      [{"total_count": 2, "artifacts": [row, row]}],
                      [{"total_count": 1, "artifacts": [row]}, {"total_count": 0, "artifacts": []}]):
            with self.subTest(pages=pages), self.assertRaises(release.ReleaseError):
                self.restore_absent(pages)
        self.assertFalse(self.restore_absent([{"total_count": 0, "artifacts": []}]))
        with patch.object(release, "gh", return_value=[{"total_count": 2, "artifacts": [row]},
                         {"total_count": 2, "artifacts": [{**row, "id": 7}]}]):
            self.assertEqual([v["id"] for v in release.pages("fixture", "artifacts")], [6, 7])

    def test_deleted_package_is_not_rebuilt_before_publication(self):
        empty = [{"total_count": 0, "artifacts": []}]
        for conclusion in ("success", "failure", "cancelled"):
            with self.subTest(conclusion=conclusion), self.assertRaisesRegex(release.ReleaseError, "refusing replacement"):
                self.restore_absent(empty, {1: self.preparation_history(conclusion=conclusion)}, "2")
        self.assertFalse(self.restore_absent(empty, {1: self.preparation_history()}, "2"))
        with self.assertRaisesRegex(release.ReleaseError, "refusing replacement"):
            self.restore_absent(empty, {1: self.preparation_history(conclusion="success"),
                                       2: self.preparation_history(attempt=2)}, "3")

    def test_unmeasured_prior_preparation_cannot_authorize_rebuild(self):
        wrong_sha = self.preparation_history()
        wrong_sha[0]["jobs"][0]["head_sha"] = "b" * 40
        missing_step = self.preparation_history()
        missing_step[0]["jobs"][0]["steps"] = []
        for history in (wrong_sha, missing_step, [{"total_count": 1, "jobs": []}],
                        release.ReleaseError("history unavailable")):
            with self.subTest(history=history), self.assertRaises(release.ReleaseError):
                self.restore_absent([{"total_count": 0, "artifacts": []}], {1: history}, "2")

    def test_semver_uses_npm_but_rejects_loose_coercion(self):
        for version in ("1.2.3", "1.2.3-alpha.1", "1.2.3+build.7", "1.2.3-rc.1+build.7"):
            with self.subTest(version=version):
                self.assertEqual(release.canonical_version(version), version.split("+", 1)[0])
        for version in ("01.2.3", "1.2.3.4", "1.2.3-01", "v1.2.3", "1.2.3+", "1.2.3+a..b", "--prefix=/tmp"):
            with self.subTest(version=version):
                with self.assertRaises(release.ReleaseError):
                    release.canonical_version(version)

    def test_mismatched_package_version_stops_before_pack(self):
        with self.assertRaises(release.ReleaseError):
            release.prepare(self.root / "wrong", self.source, {**self.ctx, "version": "2.0.0"})

    def test_http_errors_and_malformed_responses_are_not_absence(self):
        for status in (403, 429, 500):
            with self.subTest(status=status), patch.object(release, "command", return_value=
                    subprocess.CompletedProcess([], 1, b"", f"gh: failure (HTTP {status})".encode())):
                with self.assertRaises(release.ReleaseError):
                    release.gh("test", missing=True)
        with patch.object(release, "command", return_value=subprocess.CompletedProcess([], 1, b"", b"HTTP 404")):
            self.assertIsNone(release.gh("test", missing=True))
        with patch.object(release, "command", return_value=subprocess.CompletedProcess([], 0, b"broken", b"")):
            with self.assertRaises(release.ReleaseError):
                release.gh("test", missing=True)

    def test_registry_error_and_missing_integrity_stop(self):
        for result in (
            subprocess.CompletedProcess([], 1, b'{"error":{"code":"E403"}}', b""),
            subprocess.CompletedProcess([], 0, b"null", b""),
            subprocess.CompletedProcess([], 0, b"broken", b""),
        ):
            with self.subTest(result=result.stdout), patch.object(release, "command", return_value=result):
                with self.assertRaises(release.ReleaseError):
                    release.npm_integrity("test", "1.2.3")
        with patch.object(release, "command", return_value=subprocess.CompletedProcess([], 1, b'{"error":{"code":"E404"}}', b"")):
            self.assertIsNone(release.npm_integrity("test", "1.2.3"))

    def test_artifact_download_uses_the_actions_api_media_type(self):
        def transport(args, **kwargs):
            if "Accept: application/octet-stream" in args:
                return subprocess.CompletedProcess(args, 1, b"", b"HTTP 415")
            return subprocess.CompletedProcess(args, 0, b"archive-bytes", b"")
        with patch.object(release, "command", side_effect=transport):
            self.assertEqual(release.gh("repos/example/repo/actions/artifacts/5/zip", raw=True), b"archive-bytes")

    def test_build_metadata_survives_packaging(self):
        ctx = {**self.ctx, "version": "1.2.3+build.7", "tag": "v1.2.3+build.7"}
        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp) / "source"
            source.mkdir()
            for path in self.source.iterdir():
                (source / path.name).write_bytes(path.read_bytes())
            package = json.loads((source / "package.json").read_text())
            package["version"] = ctx["version"]
            (source / "package.json").write_text(json.dumps(package))
            folder = Path(tmp) / "saved"
            release.prepare(folder, source, ctx)
            plan = release.validate(folder, ctx)
            self.assertEqual(plan["version"], "1.2.3+build.7")
            self.assertEqual(plan["registry_version"], "1.2.3")


if __name__ == "__main__":
    unittest.main()
