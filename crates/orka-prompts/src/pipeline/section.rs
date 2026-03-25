use async_trait::async_trait;
use orka_core::Result;

use super::builder::BuildContext;

/// Trait for a section of a system prompt.
///
/// Sections are composable units that generate part of a prompt.
/// Each section can decide whether to render based on the build context.
#[async_trait]
pub trait PromptSection: Send + Sync {
    /// Returns the section identifier.
    fn name(&self) -> &str;

    /// Returns true if this section must be included even when empty.
    fn is_required(&self) -> bool;

    /// Renders the section content.
    ///
    /// Returns `Ok(None)` if the section should be skipped (e.g., no content
    /// available).
    async fn render(&self, ctx: &BuildContext) -> Result<Option<String>>;
}

/// A static section with fixed content.
pub struct StaticSection {
    name: String,
    content: String,
    required: bool,
}

impl StaticSection {
    /// Create a new static section.
    pub fn new(name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            content: content.into(),
            required: false,
        }
    }

    /// Make this section required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

#[async_trait]
impl PromptSection for StaticSection {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_required(&self) -> bool {
        self.required
    }

    async fn render(&self, _ctx: &BuildContext) -> Result<Option<String>> {
        if self.content.is_empty() && !self.required {
            Ok(None)
        } else {
            Ok(Some(self.content.clone()))
        }
    }
}

/// A dynamic section that generates content based on a template.
pub struct DynamicSection<F> {
    name: String,
    template_name: String,
    generator: F,
}

impl<F> DynamicSection<F> {
    /// Create a new dynamic section.
    pub fn new(name: impl Into<String>, template_name: impl Into<String>, generator: F) -> Self {
        Self {
            name: name.into(),
            template_name: template_name.into(),
            generator,
        }
    }
}

#[async_trait]
impl<F, Fut> PromptSection for DynamicSection<F>
where
    F: Fn(&BuildContext) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<Option<serde_json::Value>>> + Send,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn is_required(&self) -> bool {
        false
    }

    async fn render(&self, ctx: &BuildContext) -> Result<Option<String>> {
        let Some(data) = (self.generator)(ctx).await? else {
            return Ok(None);
        };

        // Use template engine from context if available
        if let Some(registry) = &ctx.template_registry {
            let content = registry.render(&self.template_name, &data).await?;
            if content.is_empty() {
                Ok(None)
            } else {
                Ok(Some(content))
            }
        } else {
            // Fallback: use data as-is (simple string conversion)
            let content = data
                .as_str()
                .map(String::from)
                .or_else(|| serde_json::to_string(&data).ok())
                .unwrap_or_default();

            if content.is_empty() {
                Ok(None)
            } else {
                Ok(Some(content))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_static_section() {
        let section = StaticSection::new("test", "Hello, World!");
        let ctx = BuildContext::default();

        assert_eq!(section.name(), "test");
        assert!(!section.is_required());

        let result = section.render(&ctx).await.unwrap();
        assert_eq!(result, Some("Hello, World!".to_string()));
    }

    #[tokio::test]
    async fn test_static_section_empty() {
        let section = StaticSection::new("test", "");
        let ctx = BuildContext::default();

        let result = section.render(&ctx).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_static_section_required_empty() {
        let section = StaticSection::new("test", "").required();
        let ctx = BuildContext::default();

        assert!(section.is_required());
        let result = section.render(&ctx).await.unwrap();
        assert_eq!(result, Some("".to_string()));
    }
}
