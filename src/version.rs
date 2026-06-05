/// Human-facing release version. The project uses calendar-build versions:
/// YYYY.M.D.n, where `n` is the release number for that day.
pub const DISPLAY_VERSION: &str = "2026.6.5.1";

/// Cargo and npm require SemVer, which cannot represent four numeric
/// components. Package metadata uses the closest SemVer-compatible form;
/// CLI output, release tags, and GitHub releases use `DISPLAY_VERSION`.
pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
