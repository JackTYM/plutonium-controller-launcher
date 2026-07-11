/// Types that mirror the Plutonium CDN manifest JSON.
///
/// Protocol:
///   GET https://cdn.plutonium.pw/updater/prod.json  → ProdJson
///   for each url in prod.manifests:
///     GET url  → InfoJson
///   Download each InfoJson.files entry via baseUrl + file.hash (NOT file.name).

use serde::Deserialize;

/// Root manifest: https://cdn.plutonium.pw/updater/prod.json
#[derive(Debug, Deserialize)]
pub struct ProdJson {
    /// URLs to fetch for per-file listings.
    pub manifests: Vec<String>,
    /// Relative path (from install dir) to the binary to launch after update.
    #[serde(rename = "launchTarget")]
    pub launch_target: String,
    /// Glob patterns — files matching these that are NOT in any manifest are deleted.
    /// Parsed but not yet acted on (M5: implement Yeet cleanup).
    #[serde(default)]
    #[allow(dead_code)]
    pub yeet: Vec<String>,
}

/// Per-manifest file listing: https://cdn.plutoniummod.com/updater/prod/info.json
#[derive(Debug, Deserialize)]
pub struct InfoJson {
    pub revision: u64,
    /// Prefix for content-addressed downloads.  Append `file.hash` to get the URL.
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FileEntry {
    /// Relative path under the install dir, e.g. "launcher/assets/index.html".
    pub name: String,
    pub size: u64,
    /// Lowercase hex SHA1.
    pub hash: String,
}
