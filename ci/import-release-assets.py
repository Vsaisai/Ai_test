"""Restore previously published binaries to the matching GitLab releases."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
from urllib.parse import urlencode

api = f"{os.environ['CI_API_V4_URL']}/projects/{os.environ['CI_PROJECT_ID']}"
auth = ['--header', 'JOB-TOKEN: ' + os.environ['CI_JOB_TOKEN']]


def curl(*args):
    return subprocess.check_output(['curl', '--silent', '--show-error', '--fail',
                                    '--retry', '3', *args])


def restore(tag, name, digest, tmp):
    links_api = f'{api}/releases/{tag}/assets/links'
    url = f'{api}/packages/generic/vnshelf/{tag}/{name}'
    existing = json.loads(curl(*auth, links_api))
    for link in existing:
        if link['name'] == name:
            if link['url'] != url:
                raise ValueError(f'{tag}/{name} already points to another file')
            return
    source = f'https://github.com/Vsaisai/vnshelf-releases/releases/download/{tag}/{name}'
    curl('--location', '--proto', '=https', '--proto-redir', '=https', source, '-o', tmp)
    if hashlib.sha256(tmp.read_bytes()).hexdigest() != digest:
        raise ValueError(f'SHA-256 mismatch for {name}')
    # A retry after an interrupted link creation can reuse the uploaded package.
    status = subprocess.check_output(['curl', '--silent', '--show-error', '--head', *auth,
                                      '--output', '/dev/null', '--write-out', '%{http_code}', url]).decode()
    if status == '404':
        curl(*auth, '--upload-file', tmp, url)
    elif status not in ('200', '302', '307'):
        raise RuntimeError(f'Cannot check {name}: HTTP {status}')
    # Download via GitLab and validate before attaching. On redirects never forward the job token.
    headers = curl(*auth, '--dump-header', '-', '--output', tmp, url).decode()
    redirect = next((h.split(':', 1)[1].strip() for h in headers.splitlines()
                     if h.lower().startswith('location:')), None)
    if redirect:
        curl('--location', '--proto', '=https', '--proto-redir', '=https', redirect, '-o', tmp)
    if hashlib.sha256(tmp.read_bytes()).hexdigest() != digest:
        raise ValueError(f'GitLab package checksum mismatch for {name}')
    curl(*auth, '--data', urlencode(dict(name=name, url=url, direct_asset_path='/'+name,
                                         link_type='package')), links_api)
    print(f'{tag}: verified {name}', flush=True)


if __name__ == '__main__':
    with tempfile.TemporaryDirectory() as directory:
        for line in Path(__file__).with_name('legacy-assets.sha256').read_text().splitlines():
            restore(*line.split(), Path(directory) / 'asset')
