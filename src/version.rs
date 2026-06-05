/// Human-facing release version. The project uses calendar-build versions:
/// YYYY.M.D.n, where `n` is the release number for that day.
pub const DISPLAY_VERSION: &str = "2026.6.5.2";

/// Cargo and npm require SemVer, which cannot represent four numeric
/// components. Package metadata encodes the daily build into the patch
/// number as `YYYY.M.DNN`, so `2026.6.5.2` becomes `2026.6.502`.
/// CLI output, release tags, and GitHub releases use `DISPLAY_VERSION`.
pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
