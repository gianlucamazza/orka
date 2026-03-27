/// Terminal color capability level, detected from environment variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorLevel {
    /// No color output: `NO_COLOR` set or `TERM=dumb`.
    None,
    /// 16-color ANSI output (standard terminals).
    #[default]
    Basic,
    /// 256-color output (`TERM` contains `256color`).
    Color256,
    /// 24-bit true color (`COLORTERM=truecolor` or `COLORTERM=24bit`).
    TrueColor,
}

impl ColorLevel {
    /// Returns `true` when color output should be suppressed entirely.
    pub fn is_none(self) -> bool {
        self == ColorLevel::None
    }

    /// Returns `true` for 256-color or true-color capable terminals.
    pub fn supports_256(self) -> bool {
        matches!(self, ColorLevel::Color256 | ColorLevel::TrueColor)
    }
}

/// Detected terminal capabilities, computed once at startup.
#[derive(Debug, Clone)]
pub struct TermCaps {
    /// Detected color depth.
    pub color: ColorLevel,
}

impl TermCaps {
    /// Detect capabilities from environment variables.
    ///
    /// Detection order:
    /// 1. `NO_COLOR` env var present → [`ColorLevel::None`]
    /// 2. `TERM=dumb` → [`ColorLevel::None`]
    /// 3. `COLORTERM=truecolor` or `COLORTERM=24bit` → [`ColorLevel::TrueColor`]
    /// 4. `TERM` contains `256color` → [`ColorLevel::Color256`]
    /// 5. Otherwise → [`ColorLevel::Basic`]
    pub fn detect() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some();
        let term_dumb = std::env::var("TERM").ok().as_deref() == Some("dumb");

        if no_color || term_dumb {
            // Suppress colored crate output globally too
            colored::control::set_override(false);
            return Self {
                color: ColorLevel::None,
            };
        }

        let colorterm = std::env::var("COLORTERM").ok().map(|s| s.to_lowercase());
        let term = std::env::var("TERM").ok().unwrap_or_default();

        let color =
            if colorterm.as_deref() == Some("truecolor") || colorterm.as_deref() == Some("24bit") {
                ColorLevel::TrueColor
            } else if term.contains("256color") {
                ColorLevel::Color256
            } else {
                ColorLevel::Basic
            };

        Self { color }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_level_none_is_none() {
        assert!(ColorLevel::None.is_none());
        assert!(!ColorLevel::Basic.is_none());
        assert!(!ColorLevel::TrueColor.is_none());
    }

    #[test]
    fn color_level_supports_256() {
        assert!(!ColorLevel::None.supports_256());
        assert!(!ColorLevel::Basic.supports_256());
        assert!(ColorLevel::Color256.supports_256());
        assert!(ColorLevel::TrueColor.supports_256());
    }
}
