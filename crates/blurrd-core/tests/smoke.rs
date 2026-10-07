//! Smoke test: the crate links and exposes the workspace version.

#[test]
fn version_is_semver() {
    let version = blurrd_core::VERSION;
    let parts: Vec<&str> = version.split('.').collect();
    assert_eq!(parts.len(), 3, "expected MAJOR.MINOR.PATCH, got {version}");
    assert!(
        parts.iter().all(|part| part.parse::<u64>().is_ok()),
        "non-numeric part in {version}"
    );
}
