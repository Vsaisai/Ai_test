"""Attach the current tag's verified build outputs to its GitLab release."""
import hashlib
import json
import os
import re
import subprocess
import tempfile
from pathlib import Path

tag = os.environ.get("CI_COMMIT_TAG")
if not tag and os.environ.get("CI_COMMIT_BRANCH") == "release/mobile-0.4.10":
    tag = "mobile-v0.4.10"
match = re.fullmatch(r"(pc|mobile)-v(\d+(?:\.\d+){2,3})", tag)
if not match:
    raise SystemExit(f"Unexpected release tag: {tag}")
channel, version = match.groups()
mobile = channel == "mobile"
source = Path("vndb-explorer")
version_file = source / ("mobile/VERSION" if mobile else "VERSION")
if version_file.read_text().strip() != version:
    raise SystemExit(f"Tag {tag} does not match {version_file}")
api = f"{os.environ['CI_API_V4_URL']}/projects/{os.environ['CI_PROJECT_ID']}"
package = f"{api}/packages/generic/vnshelf-{channel}/{version}"
files = ([source / f"target/apk/VNShelf-{version}.apk"] if mobile else
         [source / f"target/dist/VNShelf-{version}-{system}.zip" for system in ("windows-x64", "linux-x64")])
if not all(path.is_file() and path.stat().st_size for path in files):
    raise SystemExit("Missing release build outputs")


def request(url, target, *args, allow_missing=False):
    # curl receives the credential through the environment; no shell or URL interpolation.
    result = subprocess.run(["curl", "--silent", "--show-error", "--output", str(target),
                             "--write-out", "%{http_code}", "--header",
                             f"JOB-TOKEN: {os.environ['CI_JOB_TOKEN']}", *args, url],
                            capture_output=True, text=True, check=True)
    status = int(result.stdout)
    if not 200 <= status < 300 and not (allow_missing and status == 404):
        raise SystemExit(f"GitLab returned HTTP {status} for {url}")
    return status


def json_write(url, target, obj):
    request(url, target, "--header", "Content-Type: application/json", "--data", json.dumps(obj))


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").digest()


with tempfile.TemporaryDirectory() as work:
    response = Path(work) / "response"
    links = []
    for path in files:
        url = f"{package}/{path.name}"
        # The package files API exposes server-side SHA-256; avoid downloading a signed storage URL.
        packages_url = f"{api}/packages?package_type=generic&package_name=vnshelf-{channel}&package_version={version}&per_page=100"
        request(packages_url, response)
        packages = [p for p in json.loads(response.read_bytes())
                    if p["name"] == f"vnshelf-{channel}" and p["version"] == version]
        existing = []
        for pkg in packages:
            request(f"{api}/packages/{pkg['id']}/package_files?per_page=100", response)
            existing.extend(f for f in json.loads(response.read_bytes()) if f["file_name"] == path.name)
        if existing:
            if any(f["file_sha256"] != digest(path).hex() for f in existing):
                raise SystemExit(f"Different package already exists: {path.name}")
        else:
            request(url, response, "--upload-file", str(path))
        links.append({"name": path.name, "url": url, "direct_asset_path": "/" + path.name, "link_type": "package"})

    release = f"{api}/releases/{tag}"
    if request(release, response, allow_missing=True) == 404:
        notes = source / ("mobile" if mobile else "desktop") / "RELEASE_NOTES.md"
        json_write(f"{api}/releases", response, {"name": f"VN Shelf {version} ({channel})", "tag_name": tag, "ref": os.environ["CI_COMMIT_SHA"],
                   "description": notes.read_text(), "assets": {"links": links}})
    else:
        published = {item["name"]: item for item in json.loads(response.read_bytes())["assets"]["links"]}
        for link in links:
            prior = published.get(link["name"])
            if not prior:
                json_write(release + "/assets/links", response, link)
            elif prior["url"] != link["url"] or not prior.get("direct_asset_url", "").endswith("/downloads/" + link["name"]):
                raise SystemExit(f"Conflicting release link: {link['name']}")
