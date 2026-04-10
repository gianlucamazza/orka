//! Conversation export: JSON, Markdown, and PDF formats.

use std::fmt::Write as _;

use orka_core::{Conversation, ConversationMessage, ConversationMessageRole};
use printpdf::{BuiltinFont, Mm, PdfDocument};

/// Supported export formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// JSON format — full structured export.
    Json,
    /// Markdown format — human-readable transcript.
    Markdown,
    /// PDF format — printable document.
    Pdf,
}

impl ExportFormat {
    /// Parse a format string (`json`, `md`, `markdown`, `pdf`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "json" => Some(Self::Json),
            "md" | "markdown" => Some(Self::Markdown),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }

    /// HTTP Content-Type header value for this format.
    pub fn content_type(&self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Markdown => "text/markdown; charset=utf-8",
            Self::Pdf => "application/pdf",
        }
    }

    /// File extension for this format (without the leading dot).
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Markdown => "md",
            Self::Pdf => "pdf",
        }
    }
}

/// Render a conversation as JSON bytes.
pub fn export_json(
    conversation: &Conversation,
    messages: &[ConversationMessage],
) -> serde_json::Result<Vec<u8>> {
    let doc = serde_json::json!({
        "id": conversation.id,
        "title": conversation.title,
        "status": conversation.status,
        "created_at": conversation.created_at,
        "updated_at": conversation.updated_at,
        "messages": messages,
    });
    serde_json::to_vec_pretty(&doc)
}

/// Render a conversation as Markdown bytes.
pub fn export_markdown(conversation: &Conversation, messages: &[ConversationMessage]) -> Vec<u8> {
    let mut out = String::new();
    let _ = writeln!(out, "# {}\n", conversation.title);
    let _ = writeln!(
        out,
        "_Exported: {}_\n",
        conversation.updated_at.format("%Y-%m-%d %H:%M UTC")
    );
    out.push_str("---\n\n");
    for message in messages {
        let role_label = match message.role {
            ConversationMessageRole::User => "**User**",
            ConversationMessageRole::Assistant => "**Assistant**",
            _ => "**Other**",
        };
        let _ = writeln!(out, "### {role_label}\n");
        out.push_str(&message.text);
        out.push_str("\n\n---\n\n");
    }
    out.into_bytes()
}

/// Page geometry constants (A4, in mm).
const PAGE_W: f32 = 210.0;
const PAGE_H: f32 = 297.0;
const MARGIN: f32 = 20.0;
/// Line height in mm.
const LINE_H: f32 = 5.5;
/// Maximum characters per line before wrapping.
const CHARS_PER_LINE: usize = 90;

/// Render a conversation as PDF bytes.
///
/// Uses Helvetica (a PDF built-in font; no embedding required). Non-Latin-1
/// characters are replaced with `?`.
pub fn export_pdf(
    conversation: &Conversation,
    messages: &[ConversationMessage],
) -> Result<Vec<u8>, String> {
    let title = sanitize_pdf_string(&conversation.title);
    let (doc, page1, layer1) = PdfDocument::new(&title, Mm(PAGE_W), Mm(PAGE_H), "Layer 1");

    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| format!("font error: {e}"))?;
    let font_bold = doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .map_err(|e| format!("font error: {e}"))?;

    let mut layer = doc.get_page(page1).get_layer(layer1);
    let mut y: f32 = PAGE_H - MARGIN;

    // Draw title.
    layer.use_text(&title, 16.0, Mm(MARGIN), Mm(y), &font_bold);
    y -= LINE_H * 1.8;
    if y < MARGIN {
        let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Layer");
        layer = doc.get_page(p).get_layer(l);
        y = PAGE_H - MARGIN;
    }

    // Draw messages.
    for message in messages {
        let role_label = match message.role {
            ConversationMessageRole::User => "User",
            ConversationMessageRole::Assistant => "Assistant",
            _ => "Other",
        };
        layer.use_text(role_label, 10.0, Mm(MARGIN), Mm(y), &font_bold);
        y -= LINE_H;
        if y < MARGIN {
            let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Layer");
            layer = doc.get_page(p).get_layer(l);
            y = PAGE_H - MARGIN;
        }

        for line in wrap_text(&message.text, CHARS_PER_LINE) {
            let safe_line = sanitize_pdf_string(&line);
            layer.use_text(&safe_line, 10.0, Mm(MARGIN), Mm(y), &font);
            y -= LINE_H;
            if y < MARGIN {
                let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Layer");
                layer = doc.get_page(p).get_layer(l);
                y = PAGE_H - MARGIN;
            }
        }

        // Gap between messages.
        y -= LINE_H * 0.5;
        if y < MARGIN {
            let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Layer");
            layer = doc.get_page(p).get_layer(l);
            y = PAGE_H - MARGIN;
        }
    }

    doc.save_to_bytes()
        .map_err(|e| format!("PDF save error: {e}"))
}

/// Replace non-Latin-1 characters with `?` so Helvetica can render them.
fn sanitize_pdf_string(s: &str) -> String {
    s.chars()
        .map(|c| if (c as u32) < 256 { c } else { '?' })
        .collect()
}

/// Wrap `text` to at most `max_chars` per line, splitting on word boundaries.
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.len() + 1 + word.len() <= max_chars {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current.clone());
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}
