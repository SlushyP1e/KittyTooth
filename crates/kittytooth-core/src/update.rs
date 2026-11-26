//! In-app update decisions that don't need Tauri: version comparison, how this copy was
//! installed (and so whether it can update itself), and the GitHub release page links.
//!
//! The desktop shell does the network work with `tauri-plugin-updater` and asks this module
//! what to do with the result.

use std::path::Path;

use semver::Version;
use serde::Serialize;

/// Where releases are published. `latest.json` for the updater lives under `latest/download`.
pub const RELEASES_URL: &str = "https://github.com/SlushyP1e/KittyTooth/releases";

/// The GitHub release page for `version` ("1.3.0" or "v1.3.0").
pub fn release_page_url(version: &str) -> String {
    let v = version.trim().trim_start_matches(['v', 'V']);
    if v.is_empty() {
        format!("{RELEASES_URL}/latest")
    } else {
        format!("{RELEASES_URL}/tag/v{v}")
    }
}

/// Parses "1.2.3", "v1.2.3" or "1.2.3-beta.1". Returns `None` for anything else.
pub fn parse_version(s: &str) -> Option<Version> {
    Version::parse(s.trim().trim_start_matches(['v', 'V'])).ok()
}

/// True when `candidate` should be offered to someone running `current`.
///
/// Plain semver ordering, except that a stable build is never offered a pre-release
/// (someone on 1.2.0 doesn't get 1.3.0-beta.1, but someone on 1.3.0-beta.1 gets beta.2).
pub fn is_newer(current: &Version, candidate: &Version) -> bool {
    if !candidate.pre.is_empty() && current.pre.is_empty() {
        return false;
    }
    candidate > current
}

/// String form of [`is_newer`]; unparseable versions are never newer.
pub fn is_newer_str(current: &str, candidate: &str) -> bool {
    match (parse_version(current), parse_version(candidate)) {
        (Some(c), Some(n)) => is_newer(&c, &n),
        _ => false,
    }
}

/// How this copy of KittyTooth was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    /// Installed with the Windows setup.exe (NSIS); the updater runs the new installer.
    Installer,
    /// Running as a Linux AppImage; the updater swaps the AppImage file.
    AppImage,
    /// The portable exe, a dev build, or anything else the updater can't replace in place.
    /// These get a link to the release page instead.
    Portable,
}

impl InstallKind {
    /// Whether "Install and restart" can work for this copy.
    pub fn can_self_update(self) -> bool {
        !matches!(self, InstallKind::Portable)
    }
}

/// The NSIS installer drops this next to the exe; the portable exe has nothing beside it.
pub const NSIS_UNINSTALLER: &str = "uninstall.exe";

/// What we know about the running copy, gathered by [`detect_install_kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallFacts {
    /// `std::env::consts::OS`.
    pub os: &'static str,
    /// The NSIS uninstaller sits next to the running exe.
    pub has_uninstaller: bool,
    /// `$APPIMAGE` is set, i.e. we were launched from an AppImage.
    pub appimage: bool,
    /// A debug build (`npm run tauri dev`); never self-update those.
    pub debug: bool,
}

/// The portable vs installed decision.
pub fn install_kind(f: InstallFacts) -> InstallKind {
    if f.debug {
        return InstallKind::Portable;
    }
    match f.os {
        "windows" if f.has_uninstaller => InstallKind::Installer,
        "linux" if f.appimage => InstallKind::AppImage,
        _ => InstallKind::Portable,
    }
}

/// Looks at the running process and decides how it was installed.
pub fn detect_install_kind() -> InstallKind {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    install_kind(InstallFacts {
        os: std::env::consts::OS,
        has_uninstaller: exe_dir.is_some_and(|d| d.join(NSIS_UNINSTALLER).is_file()),
        appimage: std::env::var_os("APPIMAGE").is_some_and(|v| !v.is_empty()),
        debug: cfg!(debug_assertions),
    })
}

/// Prefix of the placeholder public key committed before the signing key exists.
pub const PUBKEY_PLACEHOLDER: &str = "REPLACE_WITH_PUBLIC_KEY";

/// True for a real minisign public key (as printed by `npx tauri signer generate`), false for
/// the placeholder or an empty value.
pub fn is_real_pubkey(key: &str) -> bool {
    let k = key.trim();
    !k.is_empty() && !k.starts_with(PUBKEY_PLACEHOLDER)
}

/// An available update, as shown in the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UpdateInfo {
    /// The version running now.
    pub current: String,
    /// The version on offer.
    pub version: String,
    /// Release notes (the GitHub release body, Markdown as plain text).
    pub notes: Option<String>,
    /// Publish date as given by the release manifest (RFC 3339), if any.
    pub date: Option<String>,
    pub install_kind: InstallKind,
    /// False for portable copies: show the release page link instead of installing.
    pub can_install: bool,
    pub release_url: String,
}

impl UpdateInfo {
    pub fn new(
        current: &str,
        version: &str,
        notes: Option<String>,
        date: Option<String>,
        kind: InstallKind,
    ) -> Self {
        Self {
            current: current.to_string(),
            version: version.to_string(),
            notes: notes.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
            date,
            install_kind: kind,
            can_install: kind.can_self_update(),
            release_url: release_page_url(version),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(os: &'static str) -> InstallFacts {
        InstallFacts {
            os,
            has_uninstaller: false,
            appimage: false,
            debug: false,
        }
    }

    #[test]
    fn newer_versions() {
        assert!(is_newer_str("1.2.0", "1.2.1"));
        assert!(is_newer_str("1.2.0", "1.3.0"));
        assert!(is_newer_str("1.2.0", "2.0.0"));
        assert!(is_newer_str("1.9.0", "1.10.0"), "numeric, not lexical");
        assert!(is_newer_str("v1.2.0", "V1.2.1"), "tolerates a v prefix");
    }

    #[test]
    fn same_or_older_is_not_newer() {
        assert!(!is_newer_str("1.2.0", "1.2.0"));
        assert!(!is_newer_str("1.2.0", "1.1.9"));
        assert!(!is_newer_str("2.0.0", "1.99.99"));
    }

    #[test]
    fn prereleases() {
        assert!(!is_newer_str("1.2.0", "1.3.0-beta.1"), "stable never gets a beta");
        assert!(is_newer_str("1.3.0-beta.1", "1.3.0-beta.2"));
        assert!(
            is_newer_str("1.3.0-beta.2", "1.3.0"),
            "beta moves on to the release"
        );
        assert!(!is_newer_str("1.3.0", "1.3.0-rc.1"));
    }

    #[test]
    fn garbage_is_never_newer() {
        assert!(!is_newer_str("1.2.0", ""));
        assert!(!is_newer_str("1.2.0", "latest"));
        assert!(!is_newer_str("1.2", "1.3"));
        assert!(!is_newer_str("nope", "1.3.0"));
    }

    #[test]
    fn windows_installer_vs_portable() {
        let installed = InstallFacts {
            has_uninstaller: true,
            ..facts("windows")
        };
        assert_eq!(install_kind(installed), InstallKind::Installer);
        assert_eq!(install_kind(facts("windows")), InstallKind::Portable);
        assert!(install_kind(installed).can_self_update());
        assert!(!install_kind(facts("windows")).can_self_update());
    }

    #[test]
    fn linux_appimage_vs_other() {
        let appimage = InstallFacts {
            appimage: true,
            ..facts("linux")
        };
        assert_eq!(install_kind(appimage), InstallKind::AppImage);
        assert!(install_kind(appimage).can_self_update());
        assert_eq!(install_kind(facts("linux")), InstallKind::Portable);
    }

    #[test]
    fn debug_builds_and_other_platforms_never_self_update() {
        let dev = InstallFacts {
            has_uninstaller: true,
            debug: true,
            ..facts("windows")
        };
        assert_eq!(install_kind(dev), InstallKind::Portable);
        let mac = InstallFacts {
            has_uninstaller: true,
            appimage: true,
            ..facts("macos")
        };
        assert_eq!(install_kind(mac), InstallKind::Portable);
    }

    #[test]
    fn placeholder_pubkey_is_not_real() {
        assert!(!is_real_pubkey(""));
        assert!(!is_real_pubkey("  "));
        assert!(!is_real_pubkey(
            "REPLACE_WITH_PUBLIC_KEY_FROM_npx_tauri_signer_generate"
        ));
        assert!(is_real_pubkey(
            "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6"
        ));
    }

    #[test]
    fn release_links() {
        assert_eq!(release_page_url("1.3.0"), format!("{RELEASES_URL}/tag/v1.3.0"));
        assert_eq!(release_page_url("v1.3.0"), format!("{RELEASES_URL}/tag/v1.3.0"));
        assert_eq!(release_page_url(""), format!("{RELEASES_URL}/latest"));
    }

    #[test]
    fn portable_info_points_at_the_release_page() {
        let i = UpdateInfo::new("1.2.0", "1.3.0", Some("  \n".into()), None, InstallKind::Portable);
        assert!(!i.can_install);
        assert_eq!(i.notes, None, "blank notes are dropped");
        assert!(i.release_url.ends_with("/tag/v1.3.0"));
        let j = UpdateInfo::new(
            "1.2.0",
            "1.3.0",
            Some("Fixes\n".into()),
            None,
            InstallKind::Installer,
        );
        assert!(j.can_install);
        assert_eq!(j.notes.as_deref(), Some("Fixes"));
        let v = serde_json::to_value(&j).unwrap();
        assert_eq!(v["install_kind"], "installer");
    }
}
