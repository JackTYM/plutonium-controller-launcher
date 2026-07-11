/// Patch launcher/assets/index.html to inject our controller-nav script,
/// and write controller-nav.js alongside it.
///
/// Strategy: always overwrite these two files with our versions on every run,
/// regardless of what the CDN says.  The updater skips downloads only when the
/// local SHA1 matches the manifest hash; since our patched index.html won't
/// match, we handle it by intercepting the file *before* the hash check.
///
/// The caller (updater) calls `is_patched_file(name)` to skip the CDN download
/// for files we manage here, then calls `write_patched_files(install_dir)` once
/// all other files are synced.

use std::fs;
use std::path::Path;
use anyhow::{Context, Result};

/// Source of the controller-nav script, embedded at compile time.
const CONTROLLER_NAV_JS: &str = include_str!("../assets/controller-nav.js");

/// Files we intercept and manage ourselves.
const PATCHED_FILES: &[&str] = &[
    "launcher/assets/index.html",
    "launcher/assets/controller-nav.js",
];

/// Returns true if the file should NOT be downloaded from the CDN.
pub fn is_patched_file(name: &str) -> bool {
    PATCHED_FILES.contains(&name)
}

/// Write our patched index.html and controller-nav.js into the install dir.
/// `original_html` must be the *stock* bytes from the CDN manifest — we parse
/// it here instead of shipping a fork, so future Plutonium HTML updates are
/// automatically preserved and only our one injection needs re-checking.
pub fn write_patched_files(install_dir: &Path, original_html: &str) -> Result<()> {
    let assets_dir = install_dir.join("launcher").join("assets");
    fs::create_dir_all(&assets_dir)
        .with_context(|| format!("create {}", assets_dir.display()))?;

    // Inject our script tag just before </body>.
    let injected = inject_script_tag(original_html);
    fs::write(assets_dir.join("index.html"), injected.as_bytes())
        .context("write patched index.html")?;

    // Always write current embedded controller-nav.js.
    fs::write(assets_dir.join("controller-nav.js"), CONTROLLER_NAV_JS.as_bytes())
        .context("write controller-nav.js")?;

    Ok(())
}

/// Insert `<script src="controller-nav.js"></script>` immediately before `</body>`.
/// If the tag is already present (re-run without a CDN update), it is not doubled.
fn inject_script_tag(html: &str) -> String {
    const TAG: &str = r#"<script src="controller-nav.js"></script>"#;

    if html.contains(TAG) {
        return html.to_owned();
    }

    // Insert before </body> so it runs after the Vue bundle is loaded.
    if let Some(pos) = html.rfind("</body>") {
        let mut out = String::with_capacity(html.len() + TAG.len() + 1);
        out.push_str(&html[..pos]);
        out.push_str(TAG);
        out.push_str(&html[pos..]);
        out
    } else {
        // Fallback: append at end (unusual, but don't silently drop the tag).
        format!("{}{}", html, TAG)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_before_body_close() {
        let html = r#"<html><body><div id="app"></div></body></html>"#;
        let out = inject_script_tag(html);
        assert!(out.contains(r#"<script src="controller-nav.js"></script></body>"#));
    }

    #[test]
    fn does_not_double_inject() {
        let html = r#"<body><script src="controller-nav.js"></script></body>"#;
        let out = inject_script_tag(html);
        let count = out.matches(r#"controller-nav.js"#).count();
        assert_eq!(count, 1);
    }
}
