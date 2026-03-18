use orka_core::Result;

use super::DocumentParser;

/// Parser that extracts plain text from PDF documents using `pdf-extract`.
pub struct PdfParser;

impl DocumentParser for PdfParser {
    fn parse(&self, data: &[u8]) -> Result<String> {
        pdf_extract::extract_text_from_mem(data)
            .map_err(|e| orka_core::Error::Knowledge(format!("PDF parse error: {e}")))
    }
}
