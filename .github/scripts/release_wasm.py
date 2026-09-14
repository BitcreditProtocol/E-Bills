#!/usr/bin/env python3
"""Prepare immutable WASM bytes and reconcile one native Actions run."""

import base64
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from urllib.parse import quote
import zipfile

ARTIFACT = "release-package"
REGISTRY = "https://registry.npmjs.org"
PLAN = "release-plan.json"


class ReleaseError(RuntimeError):
    pass


def command(args, **kwargs):
    return subprocess.run(args, capture_output=True, **kwargs)


def gh(endpoint, *, method="GET", data=None, missing=False, raw=False, paginate=False,
       accept="application/vnd.github+json"):
    args = ["gh", "api", endpoint, "--method", method, "-H", "Accept: " + accept]
    if paginate:
        args += ["--paginate", "--slurp"]
    payload = None
    if data is not None:
        args += ["--input", "-"]
        payload = json.dumps(data).encode()
    result = command(args, input=payload)
    if result.returncode:
        status = re.search(rb"HTTP (\d+)", result.stderr)
        status = status[1].decode() if status else "unknown"
        if missing and status == "404":
            return None
        raise ReleaseError(f"GitHub {method} failed (HTTP {status})")
    if raw:
        return result.stdout
    try:
        return json.loads(result.stdout)
    except (ValueError, TypeError) as error:
        raise ReleaseError("GitHub returned invalid JSON") from error


def pages(endpoint, key=None):
    result = gh(endpoint + "?per_page=100", paginate=True)
    if not isinstance(result, list) or not result:
        raise ReleaseError("Invalid paginated response")
    values, total = [], None
    for page in result:
        if key:
            if not isinstance(page, dict) or type(page.get("total_count")) is not int or page["total_count"] < 0:
                raise ReleaseError("Invalid paginated count")
            if total is not None and total != page["total_count"]:
                raise ReleaseError("Paginated count changed during reading")
            total = page["total_count"]
        rows = page.get(key) if key and isinstance(page, dict) else page
        if not isinstance(rows, list) or len(rows) > 100 or not all(isinstance(row, dict) for row in rows):
            raise ReleaseError("Incomplete paginated response")
        values.extend(rows)
    if key and (len(values) != total or any(type(row.get("id")) is not int or row["id"] <= 0 for row in values)
                or len({row["id"] for row in values}) != total):
        raise ReleaseError("Incomplete or duplicate paginated inventory")
    return values


def require_unsaved_preparation(ctx):
    attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "")
    if not re.fullmatch(r"[1-9][0-9]*", attempt):
        raise ReleaseError("Current run attempt is unmeasured")
    for previous in range(1, int(attempt)):
        jobs = pages(f"repos/{ctx['repository']}/actions/runs/{ctx['run_id']}/attempts/{previous}/jobs", "jobs")
        if any(type(job.get("run_id")) is not int or str(job["run_id"]) != ctx["run_id"]
               or type(job.get("run_attempt")) is not int or job["run_attempt"] != previous
               or job.get("head_sha") != ctx["sha"] or job.get("status") != "completed"
               or not isinstance(job.get("conclusion"), str) or not job["conclusion"]
               or not isinstance(job.get("name"), str) or not job["name"]
               or not isinstance(job.get("steps"), list) for job in jobs):
            raise ReleaseError("Previous preparation history is incomplete")
        owners = [job for job in jobs if job["name"] == "WASM Release and Publish"]
        if len(owners) > 1:
            raise ReleaseError("Previous preparation job is ambiguous")
        if not owners or owners[0]["conclusion"] == "skipped" and not owners[0]["steps"]:
            continue
        steps = owners[0]["steps"]
        if any(not isinstance(step, dict) or type(step.get("number")) is not int or step["number"] <= 0
               or not isinstance(step.get("name"), str) or not step["name"] for step in steps) \
                or len({step["number"] for step in steps}) != len(steps):
            raise ReleaseError("Previous preparation steps are incomplete")
        saves = [step for step in steps if step["name"] == "Save the package before any publication writes"]
        if len(saves) != 1 or not (
            saves[0].get("status") == "completed" and saves[0].get("conclusion") == "skipped"
            or saves[0].get("status") == "queued" and "conclusion" in saves[0] and saves[0]["conclusion"] is None
            and "started_at" in saves[0] and saves[0]["started_at"] is None
        ):
            raise ReleaseError("The original package may have been saved; refusing replacement bytes")


def context():
    repo, sha, run = (os.environ[name] for name in ("GITHUB_REPOSITORY", "GITHUB_SHA", "GITHUB_RUN_ID"))
    if repo != "BitcreditProtocol/Bitcredit-Core" or not re.fullmatch(r"[0-9a-f]{40}", sha) or not run.isdigit():
        raise ReleaseError("Invalid release run identity")
    version = tomllib.loads(Path("Cargo.toml").read_text())["workspace"]["package"]["version"]
    crate = tomllib.loads(Path("crates/bcr-ebill-wasm/Cargo.toml").read_text())["package"]["name"]
    return dict(repository=repo, sha=sha, run_id=run, version=version, tag="v" + version,
                package_name="@bitcredit/" + crate.replace("_", "-"))


def canonical_version(version):
    # npm validates the version; equality rejects npm's loose coercions.
    if not isinstance(version, str) or not re.fullmatch(r"[0-9][0-9A-Za-z.+-]*", version):
        raise ReleaseError("Invalid version syntax")
    base, separator, build = version.partition("+")
    if separator and not re.fullmatch(r"[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*", build):
        raise ReleaseError("Invalid SemVer build metadata")
    with tempfile.TemporaryDirectory() as tmp:
        package = Path(tmp) / "package.json"
        package.write_text('{"name":"release-version-check","version":"0.0.0"}')
        result = command(["npm", "version", version, "--allow-same-version", "--no-git-tag-version", "--ignore-scripts",
                          "--cache", str(Path(tmp) / "cache")], cwd=tmp)
        normalized = json.loads(package.read_text())["version"]
        if result.returncode or normalized != base:
            raise ReleaseError("Version is not strict SemVer")
    return normalized


def npm_integrity(name, version):
    result = command(["npm", "view", f"{name}@{version}", "dist.integrity", "--json", "--registry", REGISTRY])
    try:
        value = json.loads(result.stdout)
    except ValueError as error:
        raise ReleaseError("npm returned invalid JSON") from error
    if result.returncode:
        if isinstance(value, dict) and value.get("error", {}).get("code") == "E404":
            return None
        raise ReleaseError("npm metadata read failed")
    if not isinstance(value, str) or not value.startswith("sha512-"):
        raise ReleaseError("npm did not return a SHA-512 integrity value")
    return value


def tag_sha(ctx):
    value = gh(f"repos/{ctx['repository']}/git/ref/tags/{quote(ctx['tag'], safe='')}", missing=True)
    if value is None:
        return None
    obj = value["object"]
    for _ in range(20):
        if obj.get("type") == "commit" and re.fullmatch(r"[0-9a-f]{40}", obj.get("sha", "")):
            return obj["sha"]
        if obj.get("type") != "tag" or not re.fullmatch(r"[0-9a-f]{40}", obj.get("sha", "")):
            break
        obj = gh(f"repos/{ctx['repository']}/git/tags/{obj['sha']}")["object"]
    raise ReleaseError("Invalid or excessively nested tag")


def release(ctx):
    rows = pages(f"repos/{ctx['repository']}/releases")
    if any(type(row.get("id")) is not int or row["id"] <= 0
           or not isinstance(row.get("tag_name"), str) or not row["tag_name"]
           or type(row.get("draft")) is not bool for row in rows):
        raise ReleaseError("Invalid release list")
    found = [row for row in rows if row["tag_name"] == ctx["tag"]]
    if len(found) > 1:
        raise ReleaseError("Multiple releases match the tag")
    if not found:
        return None
    value = found[0]
    if type(value.get("id")) is not int or value["id"] <= 0 or type(value.get("draft")) is not bool:
        raise ReleaseError("Invalid release identity")
    return value


def digest(path, algorithm="sha256"):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, algorithm).digest()


def safe_file(name):
    return name == "package.tgz" or bool(re.fullmatch(r"assets/[A-Za-z0-9][A-Za-z0-9._+-]*", name))


def validate(folder, ctx):
    plan = json.loads((folder / PLAN).read_text())
    if plan.get("schema") != 1 or any(plan.get(key) != value for key, value in ctx.items()):
        raise ReleaseError("Saved package belongs to a different source or run")
    files = plan.get("files")
    if not isinstance(files, dict) or "package.tgz" not in files or not files:
        raise ReleaseError("Invalid saved file inventory")
    for name, info in files.items():
        if not safe_file(name) or not isinstance(info, dict):
            raise ReleaseError("Invalid saved package path")
        path = folder / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size != info.get("size") or digest(path).hex() != info.get("sha256"):
            raise ReleaseError(f"Saved file failed checksum validation: {name}")
    actual = {p.relative_to(folder).as_posix() for p in folder.rglob("*") if p.is_file()}
    if actual != set(files) | {PLAN}:
        raise ReleaseError("Saved package contains unexpected files")
    integrity = "sha512-" + base64.b64encode(digest(folder / "package.tgz", "sha512")).decode()
    if plan.get("integrity") != integrity or plan.get("registry_version") != canonical_version(ctx["version"]):
        raise ReleaseError("Invalid package integrity or registry version")
    with tarfile.open(folder / "package.tgz", "r:gz") as archive:
        package = json.load(archive.extractfile("package/package.json"))
        for name in ("index.js", "index.d.ts", "index_bg.wasm"):
            member = archive.getmember("package/" + name)
            if not member.isfile() or member.size == 0:
                raise ReleaseError("Required WASM package file is missing or empty")
    if package.get("name") != ctx["package_name"] or package.get("version") != ctx["version"]:
        raise ReleaseError("Saved npm manifest does not match source metadata")
    return plan


def restore(folder, ctx):
    rows = pages(f"repos/{ctx['repository']}/actions/runs/{ctx['run_id']}/artifacts", "artifacts")
    if any(type(row.get("id")) is not int or row["id"] <= 0
           or not isinstance(row.get("name"), str) or not row["name"]
           or type(row.get("expired")) is not bool for row in rows):
        raise ReleaseError("Invalid artifact list")
    candidates = [a for a in rows if a["name"] == ARTIFACT]
    if not candidates:
        version = canonical_version(ctx["version"])
        # Without saved bytes, existing external state must never be adopted.
        if tag_sha(ctx) is not None or release(ctx) is not None or npm_integrity(ctx["package_name"], version) is not None:
            raise ReleaseError("Publication exists but the immutable package artifact is missing")
        require_unsaved_preparation(ctx)
        return False
    if len(candidates) != 1:
        raise ReleaseError("Ambiguous package artifact")
    artifact = candidates[0]
    if artifact.get("expired") is not False or type(artifact.get("id")) is not int or artifact["id"] <= 0:
        raise ReleaseError("Package artifact is expired or invalid")
    payload = gh(f"repos/{ctx['repository']}/actions/artifacts/{artifact['id']}/zip", raw=True)
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        names = [item.filename for item in archive.infolist() if not item.is_dir()]
        if len(names) != len(set(names)) or PLAN not in names or any(name != PLAN and not safe_file(name) for name in names):
            raise ReleaseError("Unsafe or invalid artifact archive")
        folder.mkdir(parents=True, exist_ok=True)
        for name in names:
            destination = folder / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(archive.read(name))
    validate(folder, ctx)
    note(f"Restored artifact {artifact['id']} from original run {ctx['run_id']}.")
    return True


def prepare(folder, source, ctx):
    package = json.loads((source / "package.json").read_text())
    if package.get("version") != ctx["version"] or package.get("name") != ctx["package_name"]:
        raise ReleaseError("Built WASM package does not match the source version/name")
    version = canonical_version(ctx["version"])
    folder.mkdir(parents=True, exist_ok=False)
    (folder / "assets").mkdir()
    for path in source.iterdir():
        if path.name.startswith("."):
            continue
        if not path.is_file() or not safe_file("assets/" + path.name):
            raise ReleaseError("Unexpected WASM package entry")
        shutil.copyfile(path, folder / "assets" / path.name)
    result = command(["npm", "pack", "--json", "--ignore-scripts", "--pack-destination", str(folder.resolve())], cwd=source)
    if result.returncode:
        raise ReleaseError("npm pack failed")
    output = json.loads(result.stdout)
    name = output[0]["filename"]
    if Path(name).name != name:
        raise ReleaseError("Invalid npm archive name")
    (folder / name).rename(folder / "package.tgz")
    files = {p.relative_to(folder).as_posix(): {"size": p.stat().st_size, "sha256": digest(p).hex()}
             for p in folder.rglob("*") if p.is_file()}
    plan = dict(schema=1, **ctx, registry_version=version, files=files,
                integrity="sha512-" + base64.b64encode(digest(folder / "package.tgz", "sha512")).decode(),
                changelog=os.environ.get("RELEASE_CHANGELOG", ""))
    (folder / PLAN).write_text(json.dumps(plan, indent=2, sort_keys=True) + "\n")
    validate(folder, ctx)
    note(f"Prepared {ctx['tag']} from {ctx['sha']}; no publication writes performed.")


def note(message):
    print(message)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            stream.write("- " + message + "\n")


def write_once(action, readback, description):
    # A lost response is resolved by reading state, never by blind repeat writes.
    try:
        action()
    except ReleaseError:
        pass
    if not readback():
        raise ReleaseError(f"{description} was not confirmed; rerun the original run")
    note(description + " confirmed.")


def matching_asset(ctx, release_id, name, info):
    rows = pages(f"repos/{ctx['repository']}/releases/{release_id}/assets")
    if any(type(row.get("id")) is not int or row["id"] <= 0
           or not isinstance(row.get("name"), str) or not row["name"]
           or row.get("state") not in ("starter", "uploaded")
           or type(row.get("size")) is not int or row["size"] < 0 for row in rows):
        raise ReleaseError("Invalid release asset list")
    found = [a for a in rows if a["name"] == name]
    if not found:
        return False
    if len(found) != 1:
        raise ReleaseError("Ambiguous release asset")
    asset = found[0]
    if type(asset.get("id")) is not int or asset.get("state") != "uploaded" or asset.get("size") != info["size"]:
        raise ReleaseError(f"Conflicting or incomplete release asset: {name}")
    checksum = asset.get("digest")
    if checksum is None:
        checksum = "sha256:" + hashlib.sha256(gh(f"repos/{ctx['repository']}/releases/assets/{asset['id']}", raw=True,
                                                accept="application/octet-stream")).hexdigest()
    if checksum != "sha256:" + info["sha256"]:
        raise ReleaseError(f"Conflicting release asset content: {name}")
    return True


def publish(folder, ctx):
    plan = validate(folder, ctx)
    with tempfile.TemporaryDirectory(prefix="release-readback-") as tmp:
        saved = Path(tmp)
        if not restore(saved, ctx) or validate(saved, ctx) != plan:
            raise ReleaseError("Publication bytes must match the immutable package artifact")
    current = tag_sha(ctx)
    if current is not None and current != ctx["sha"]:
        raise ReleaseError("Tag points to a different commit")
    current_npm = npm_integrity(ctx["package_name"], plan["registry_version"])
    if current_npm is not None and current_npm != plan["integrity"]:
        raise ReleaseError("npm version contains different bytes")
    existing = release(ctx)
    # Complete all initial reads before creating the first external object.
    if existing:
        for name, info in plan["files"].items():
            if name.startswith("assets/"):
                matching_asset(ctx, existing["id"], Path(name).name, info)
    if current is None:
        write_once(
            lambda: gh(f"repos/{ctx['repository']}/git/refs", method="POST",
                       data={"ref": "refs/tags/" + ctx["tag"], "sha": ctx["sha"]}),
            lambda: tag_sha(ctx) == ctx["sha"], "Saved commit tag",
        )
    if existing is None:
        body = (plan["changelog"] + f"\n\nVersion: {ctx['version']}\nSource: {ctx['sha']}\n"
                f"Candidate: https://github.com/{ctx['repository']}/actions/runs/{ctx['run_id']}\n").strip()
        write_once(
            lambda: gh(f"repos/{ctx['repository']}/releases", method="POST",
                       data={"tag_name": ctx["tag"], "name": ctx["tag"], "body": body, "draft": True}),
            lambda: release(ctx) is not None, "Draft GitHub release",
        )
        existing = release(ctx)
    for name, info in plan["files"].items():
        if not name.startswith("assets/") or matching_asset(ctx, existing["id"], Path(name).name, info):
            continue

        def upload():
            result = command(["gh", "release", "upload", ctx["tag"], str((folder / name).resolve()), "--repo", ctx["repository"]])
            if result.returncode:
                raise ReleaseError("Asset upload response failed")
        write_once(upload, lambda: matching_asset(ctx, existing["id"], Path(name).name, info), "Asset " + Path(name).name)
    prerelease = "-" in ctx["version"].split("+", 1)[0]
    if current_npm is None:
        def upload_npm():
            result = command(["npm", "publish", str((folder / "package.tgz").resolve()), "--registry", REGISTRY,
                              "--access", "public", "--provenance", "--ignore-scripts", "--tag", "next" if prerelease else "latest"])
            if result.returncode:
                raise ReleaseError("npm publication response failed")
        write_once(upload_npm, lambda: npm_integrity(ctx["package_name"], plan["registry_version"]) == plan["integrity"],
                   "npm package integrity")
    else:
        note("Existing npm package integrity matches; publication preserved.")
    if existing["draft"]:
        write_once(
            lambda: gh(f"repos/{ctx['repository']}/releases/{existing['id']}", method="PATCH",
                       data={"draft": False, "prerelease": prerelease}),
            lambda: release(ctx)["draft"] is False, "Final GitHub release",
        )
    note(f"Release {ctx['tag']} is complete for saved run {ctx['run_id']}.")


def main():
    action = sys.argv[1]
    folder = Path(sys.argv[2]).resolve()
    ctx = context()
    if action == "restore":
        restored = restore(folder, ctx)
        with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
            stream.write(f"restored={'true' if restored else 'false'}\n")
    elif action == "prepare":
        prepare(folder, Path(sys.argv[3]).resolve(), ctx)
    elif action == "publish":
        if os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch":
            raise ReleaseError("Publication requires the existing manual workflow")
        publish(folder, ctx)
    else:
        raise ReleaseError("Unknown release operation")


if __name__ == "__main__":
    try:
        main()
    except (ReleaseError, ValueError, KeyError, TypeError, OSError, zipfile.BadZipFile, tarfile.TarError) as error:
        print(f"Release stopped: {error}", file=sys.stderr)
        sys.exit(1)
