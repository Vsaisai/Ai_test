// GitLab paginates PC and Android releases together, newest first.
const project = 'https://gitlab.com/Asgefka/Ai_test';
const api = 'https://gitlab.com/api/v4/projects/88983792/releases';

export async function latestRelease(platform) {
  const perPage = 100;
  for (let page = 1; ; page++) {
    let response;
    try {
      response = await fetch(`${api}?per_page=${perPage}&page=${page}`, { credentials: 'omit' });
    } catch {
      throw new Error('Не удалось связаться с GitLab. Проверьте интернет или откройте сборки в браузере.');
    }
    if ([401, 403, 404].includes(response.status)) {
      throw new Error('Для доступа к сборкам войдите в GitLab через браузер.');
    }
    if (!response.ok) throw new Error(`GitLab: HTTP ${response.status}`);
    const releases = await response.json();
    for (const release of releases) {
      const match = /^(pc|mobile)-v(\d+(?:\.\d+){2,3})$/.exec(release.tag_name);
      if (!match || match[1] !== platform || release.upcoming_release) continue;
      const version = match[2];
      const pageUrl = `${project}/-/releases/${release.tag_name}`;
      const filename = `VNShelf-${version}.apk`;
      const assetUrl = `${pageUrl}/downloads/${filename}`;
      const linked = (release.assets?.links || []).some(link => link.direct_asset_url === assetUrl);
      return { version, page: pageUrl, url: platform === 'mobile' && linked ? assetUrl : null };
    }
    if (releases.length < perPage) return null;
  }
}
