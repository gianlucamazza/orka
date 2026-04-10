/// Opaque secret value, securely zeroized on drop.
///
/// Intentionally not `Clone` to prevent accidental copies of secrets
/// scattered across the heap. Use [`SecretValue::to_owned_secret`] for
/// explicit, deliberate copies.
pub struct SecretValue(zeroize::Zeroizing<Vec<u8>>);

impl SecretValue {
    /// Wrap raw bytes as a secret value.
    pub fn new(value: impl Into<Vec<u8>>) -> Self {
        Self(zeroize::Zeroizing::new(value.into()))
    }

    /// Access the raw secret bytes.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Access the secret as a UTF-8 string, if valid.
    pub fn expose_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// Create an explicit copy of the secret. Prefer passing references
    /// instead of cloning to minimize secret copies in memory.
    #[must_use]
    pub fn to_owned_secret(&self) -> Self {
        Self(zeroize::Zeroizing::new(self.0.to_vec()))
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// A zeroize-on-drop wrapper for secret strings (API keys, tokens, passwords).
///
/// Intentionally not `Clone` to prevent accidental copies of secrets
/// scattered across the heap. Use [`SecretStr::to_owned_secret`] for
/// explicit, deliberate copies.
pub struct SecretStr(zeroize::Zeroizing<String>);

impl SecretStr {
    /// Wrap a string as a secret.
    pub fn new(value: impl Into<String>) -> Self {
        Self(zeroize::Zeroizing::new(value.into()))
    }

    /// Access the secret string.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Create an explicit copy of the secret. Prefer passing references
    /// instead of cloning to minimize secret copies in memory.
    #[must_use]
    pub fn to_owned_secret(&self) -> Self {
        Self(self.0.clone())
    }
}

impl std::fmt::Debug for SecretStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}
