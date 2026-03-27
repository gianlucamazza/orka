use std::fmt;

use handlebars::{Handlebars, RenderError, TemplateError as HandlebarsError};
use serde::Serialize;

/// Error type for template operations.
#[derive(Debug)]
pub enum TemplateError {
    /// Template compilation failed.
    Compilation(HandlebarsError),
    /// Template rendering failed.
    Rendering(RenderError),
    /// Template not found.
    NotFound(String),
    /// Invalid template name.
    InvalidName(String),
    /// Filesystem watcher error.
    Watch(notify::Error),
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateError::Compilation(e) => write!(f, "template compilation failed: {e}"),
            TemplateError::Rendering(e) => write!(f, "template rendering failed: {e}"),
            TemplateError::NotFound(name) => write!(f, "template not found: {name}"),
            TemplateError::InvalidName(name) => write!(f, "invalid template name: {name}"),
            TemplateError::Watch(e) => write!(f, "template watcher error: {e}"),
        }
    }
}

impl From<TemplateError> for orka_core::Error {
    fn from(e: TemplateError) -> Self {
        orka_core::Error::Other(format!("template error: {e}"))
    }
}

impl std::error::Error for TemplateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TemplateError::Compilation(e) => Some(e),
            TemplateError::Rendering(e) => Some(e),
            TemplateError::Watch(e) => Some(e),
            _ => None,
        }
    }
}

impl From<HandlebarsError> for TemplateError {
    fn from(e: HandlebarsError) -> Self {
        TemplateError::Compilation(e)
    }
}

impl From<RenderError> for TemplateError {
    fn from(e: RenderError) -> Self {
        TemplateError::Rendering(e)
    }
}

impl From<notify::Error> for TemplateError {
    fn from(e: notify::Error) -> Self {
        TemplateError::Watch(e)
    }
}

/// Handlebars-based template engine for prompt rendering.
///
/// # Example
///
/// ```
/// use orka_prompts::template::TemplateEngine;
///
/// let mut engine = TemplateEngine::new();
/// engine.register_template("hello", "Hello, {{name}}!").unwrap();
///
/// let context = serde_json::json!({ "name": "World" });
/// let result = engine.render("hello", &context).unwrap();
/// assert_eq!(result, "Hello, World!");
/// ```
pub struct TemplateEngine {
    handlebars: Handlebars<'static>,
}

impl std::fmt::Debug for TemplateEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemplateEngine")
            .field(
                "templates",
                &self.handlebars.get_templates().keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Default for TemplateEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateEngine {
    /// Create a new template engine with default settings.
    pub fn new() -> Self {
        let mut hb = Handlebars::new();

        // Enable strict mode to catch missing variables
        hb.set_strict_mode(true);

        // Don't escape HTML - we're rendering markdown, not HTML
        hb.register_escape_fn(handlebars::no_escape);

        // Register built-in helpers
        Self::register_helpers(&mut hb);

        Self { handlebars: hb }
    }

    /// Register a template from a string.
    ///
    /// # Arguments
    ///
    /// * `name` - Unique template identifier (e.g., "system/reflection")
    /// * `content` - Template content in Handlebars syntax
    pub fn register_template(&mut self, name: &str, content: &str) -> Result<(), TemplateError> {
        Self::validate_name(name)?;
        self.handlebars.register_template_string(name, content)?;
        Ok(())
    }

    /// Register a template from a file path.
    pub fn register_template_file(
        &mut self,
        name: &str,
        path: &std::path::Path,
    ) -> Result<(), TemplateError> {
        Self::validate_name(name)?;
        self.handlebars
            .register_template_file(name, path)
            .map_err(TemplateError::from)
    }

    /// Render a template with the given context.
    ///
    /// # Arguments
    ///
    /// * `name` - Template name previously registered
    /// * `context` - Serializable context data
    pub fn render<C>(&self, name: &str, context: &C) -> Result<String, TemplateError>
    where
        C: Serialize,
    {
        if !self.handlebars.has_template(name) {
            return Err(TemplateError::NotFound(name.to_string()));
        }
        Ok(self.handlebars.render(name, context)?)
    }

    /// Check if a template is registered.
    pub fn has_template(&self, name: &str) -> bool {
        self.handlebars.has_template(name)
    }

    /// Remove a template from the engine.
    pub fn unregister_template(&mut self, name: &str) {
        self.handlebars.unregister_template(name);
    }

    /// Get a list of all registered template names.
    pub fn template_names(&self) -> Vec<String> {
        self.handlebars.get_templates().keys().cloned().collect()
    }

    fn validate_name(name: &str) -> Result<(), TemplateError> {
        if name.is_empty() {
            return Err(TemplateError::InvalidName("empty string".to_string()));
        }
        if name.contains('/') {
            // Allow path-like names like "system/reflection"
            Ok(())
        } else {
            Ok(())
        }
    }

    fn register_helpers(hb: &mut Handlebars) {
        // Helper to join array elements with a separator
        hb.register_helper(
            "join",
            Box::new(
                |h: &handlebars::Helper<'_>,
                 _: &handlebars::Handlebars<'_>,
                 _: &handlebars::Context,
                 _: &mut handlebars::RenderContext<'_, '_>,
                 out: &mut dyn handlebars::Output| {
                    let value = h.param(0).ok_or_else(|| {
                        let e: handlebars::RenderError =
                            handlebars::RenderErrorReason::ParamNotFoundForIndex("join", 0).into();
                        e
                    })?;
                    let separator = h
                        .param(1)
                        .map_or(", ", |p| p.value().as_str().unwrap_or(", "));

                    if let Some(arr) = value.value().as_array() {
                        let joined: Vec<String> = arr
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect();
                        out.write(&joined.join(separator))?;
                    }
                    Ok(())
                },
            ),
        );

        // Helper for 1-based index
        hb.register_helper(
            "inc",
            Box::new(
                |h: &handlebars::Helper<'_>,
                 _: &handlebars::Handlebars<'_>,
                 _: &handlebars::Context,
                 _: &mut handlebars::RenderContext<'_, '_>,
                 out: &mut dyn handlebars::Output| {
                    let value = h.param(0).ok_or_else(|| {
                        let e: handlebars::RenderError =
                            handlebars::RenderErrorReason::ParamNotFoundForIndex("inc", 0).into();
                        e
                    })?;
                    if let Some(n) = value.value().as_u64() {
                        out.write(&(n + 1).to_string())?;
                    }
                    Ok(())
                },
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_render() {
        let mut engine = TemplateEngine::new();
        engine
            .register_template("test", "Hello, {{name}}!")
            .unwrap();

        let context = serde_json::json!({ "name": "World" });
        let result = engine.render("test", &context).unwrap();
        assert_eq!(result, "Hello, World!");
    }

    #[test]
    fn test_conditional_render() {
        let mut engine = TemplateEngine::new();
        engine
            .register_template("test", "{{#if show}}visible{{else}}hidden{{/if}}")
            .unwrap();

        let context = serde_json::json!({ "show": true });
        let result = engine.render("test", &context).unwrap();
        assert_eq!(result, "visible");
    }

    #[test]
    fn test_join_helper() {
        let mut engine = TemplateEngine::new();
        engine
            .register_template("test", "Items: {{join items \", \"}}")
            .unwrap();

        let context = serde_json::json!({ "items": ["a", "b", "c"] });
        let result = engine.render("test", &context).unwrap();
        assert_eq!(result, "Items: a, b, c");
    }

    #[test]
    fn test_not_found_error() {
        let engine = TemplateEngine::new();
        let context = serde_json::json!({});
        let result = engine.render("nonexistent", &context);
        assert!(matches!(result, Err(TemplateError::NotFound(_))));
    }

    #[test]
    fn test_no_html_escaping() {
        let mut engine = TemplateEngine::new();
        engine.register_template("test", "{{content}}").unwrap();

        let context = serde_json::json!({ "content": "<script>alert('xss')</script>" });
        let result = engine.render("test", &context).unwrap();
        // Should NOT be escaped since we use no_escape
        assert_eq!(result, "<script>alert('xss')</script>");
    }
}
