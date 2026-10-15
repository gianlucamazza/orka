//! Conversation export: JSON, Markdown, and PDF formats.

use std::fmt::Write as _;

use orka_core::{Conversation, ConversationMessage, ConversationMessageRole};
use printpdf::{
    BuiltinFont, Mm, Op, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point, Pt, TextItem,
};

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
    let mut doc = PdfDocument::new(&title);
    let mut pages = Vec::new();
    let mut ops = start_page_ops();
    let mut y = PAGE_H - MARGIN;

    write_line(&mut ops, &title, BuiltinFont::HelveticaBold, 16.0, y);
    y -= LINE_H * 1.8;
    ensure_page_space(&mut pages, &mut ops, &mut y);

    for message in messages {
        let role_label = match message.role {
            ConversationMessageRole::User => "User",
            ConversationMessageRole::Assistant => "Assistant",
            _ => "Other",
        };
        write_line(&mut ops, role_label, BuiltinFont::HelveticaBold, 10.0, y);
        y -= LINE_H;
        ensure_page_space(&mut pages, &mut ops, &mut y);

        for line in wrap_text(&message.text, CHARS_PER_LINE) {
            let safe_line = sanitize_pdf_string(&line);
            write_line(&mut ops, &safe_line, BuiltinFont::Helvetica, 10.0, y);
            y -= LINE_H;
            ensure_page_space(&mut pages, &mut ops, &mut y);
        }

        y -= LINE_H * 0.5;
        ensure_page_space(&mut pages, &mut ops, &mut y);
    }

    pages.push(finish_page(ops));
    let bytes = doc
        .with_pages(pages)
        .save(&PdfSaveOptions::default(), &mut Vec::new());
    Ok(bytes)
}

fn start_page_ops() -> Vec<Op> {
    vec![Op::StartTextSection]
}

fn finish_page(mut ops: Vec<Op>) -> PdfPage {
    ops.push(Op::EndTextSection);
    PdfPage::new(Mm(PAGE_W), Mm(PAGE_H), ops)
}

fn ensure_page_space(pages: &mut Vec<PdfPage>, ops: &mut Vec<Op>, y: &mut f32) {
    if *y >= MARGIN {
        return;
    }
    pages.push(finish_page(std::mem::take(ops)));
    *ops = start_page_ops();
    *y = PAGE_H - MARGIN;
}

fn write_line(ops: &mut Vec<Op>, text: &str, font: BuiltinFont, size: f32, y: f32) {
    ops.push(Op::SetTextCursor {
        pos: Point::new(Mm(MARGIN), Mm(y)),
    });
    ops.push(Op::SetFont {
        font: PdfFontHandle::Builtin(font),
        size: Pt(size),
    });
    ops.push(Op::ShowText {
        items: vec![TextItem::Text(text.to_string())],
    });
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

#[cfg(test)]
mod tests {
    use orka_core::{
        Conversation, ConversationId, ConversationMessage, ConversationMessageRole, MessageId,
        SessionId,
    };

    use super::*;

    #[test]
    fn export_pdf_emits_pdf_header() {
        let conversation = Conversation::new(ConversationId::new(), SessionId::new(), "user", "Hi");
        let messages = [ConversationMessage::new(
            MessageId::new(),
            conversation.id,
            conversation.session_id,
            ConversationMessageRole::User,
            "hello",
        )];
        let bytes = export_pdf(&conversation, &messages).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            bytes.starts_with(b"%PDF"),
            "expected PDF magic, got {} bytes",
            bytes.len()
        );
    }
}
