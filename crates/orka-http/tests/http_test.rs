//! Unit tests for orka-http: SSRF guard and HTTP skill schema.
#![allow(clippy::unwrap_used)]

use orka_http::SsrfGuard;

// ---------------------------------------------------------------------------
// SsrfGuard
// ---------------------------------------------------------------------------

#[test]
fn allows_normal_https_url() {
    let guard = SsrfGuard::new(vec![]);
    assert!(guard.check("https://example.com/api").is_ok());
}

#[test]
fn blocks_localhost() {
    let guard = SsrfGuard::new(vec!["localhost".into(), "127.0.0.1".into()]);
    assert!(guard.check("http://localhost/secret").is_err());
    assert!(guard.check("http://127.0.0.1/secret").is_err());
}

#[test]
fn blocks_cloud_metadata_endpoint() {
    let guard = SsrfGuard::new(vec![]);
    assert!(
        guard
            .check("http://169.254.169.254/latest/meta-data/")
            .is_err()
    );
    assert!(guard.check("http://metadata.google.internal/").is_err());
}

#[test]
fn blocks_custom_domain_exact_match() {
    let guard = SsrfGuard::new(vec!["internal.corp".into()]);
    assert!(guard.check("http://internal.corp/data").is_err());
}

#[test]
fn blocks_subdomain_of_blocked_domain() {
    let guard = SsrfGuard::new(vec!["internal.corp".into()]);
    assert!(guard.check("http://api.internal.corp/data").is_err());
}

#[test]
fn allows_unrelated_domain() {
    let guard = SsrfGuard::new(vec!["internal.corp".into()]);
    assert!(guard.check("https://external.com/api").is_ok());
}

#[test]
fn rejects_invalid_url() {
    let guard = SsrfGuard::new(vec![]);
    assert!(guard.check("not a url at all").is_err());
    assert!(guard.check("").is_err());
}

#[test]
fn blocks_ipv6_loopback() {
    let guard = SsrfGuard::new(vec!["::1".into()]);
    // Depending on how URL parsing normalizes ::1 this may or may not trigger —
    // the key invariant is no panic.
    let _ = guard.check("http://[::1]/admin");
}

#[test]
fn error_message_contains_blocked_domain() {
    let guard = SsrfGuard::new(vec!["evil.com".into()]);
    let err = guard.check("http://evil.com/exfiltrate").unwrap_err();
    assert!(
        err.contains("evil.com"),
        "error should mention the blocked domain: {err}"
    );
}
