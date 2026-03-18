use url::Url;

/// SSRF protection: checks if a URL targets a blocked domain.
pub struct SsrfGuard {
    blocked_domains: Vec<String>,
}

impl SsrfGuard {
    /// Create a new guard with the given list of blocked domains.
    pub fn new(blocked_domains: Vec<String>) -> Self {
        Self { blocked_domains }
    }

    /// Returns `Err` with a reason if the URL is blocked.
    pub fn check(&self, raw_url: &str) -> Result<(), String> {
        let url = Url::parse(raw_url).map_err(|e| format!("invalid URL: {e}"))?;

        let host = url.host_str().unwrap_or("");

        // Block private/link-local IPs
        if (host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "0.0.0.0")
            && self
                .blocked_domains
                .iter()
                .any(|d| d == "localhost" || d == "127.0.0.1")
        {
            return Err("blocked: localhost access denied".into());
        }

        for blocked in &self.blocked_domains {
            if host == blocked.as_str() {
                return Err(format!("blocked: domain {host} is in the blocklist"));
            }
            // Subdomain match
            if host.ends_with(&format!(".{blocked}")) {
                return Err(format!(
                    "blocked: domain {host} matches blocklist entry {blocked}"
                ));
            }
        }

        // Block cloud metadata endpoints
        if host == "169.254.169.254" || host == "metadata.google.internal" {
            return Err("blocked: cloud metadata endpoint".into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_normal_urls() {
        let guard = SsrfGuard::new(vec!["169.254.169.254".into()]);
        assert!(guard.check("https://example.com/api").is_ok());
    }

    #[test]
    fn blocks_metadata_endpoint() {
        let guard = SsrfGuard::new(vec!["169.254.169.254".into()]);
        assert!(
            guard
                .check("http://169.254.169.254/latest/meta-data/")
                .is_err()
        );
    }

    #[test]
    fn blocks_custom_domain() {
        let guard = SsrfGuard::new(vec!["internal.corp".into()]);
        assert!(guard.check("http://internal.corp/secret").is_err());
        assert!(guard.check("http://sub.internal.corp/secret").is_err());
    }

    #[test]
    fn rejects_invalid_url() {
        let guard = SsrfGuard::new(vec![]);
        assert!(guard.check("not a url").is_err());
    }
}
