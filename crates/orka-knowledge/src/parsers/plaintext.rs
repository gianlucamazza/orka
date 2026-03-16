use orka_core::Result;

use super::DocumentParser;

pub struct PlaintextParser;

impl DocumentParser for PlaintextParser {
    fn parse(&self, data: &[u8]) -> Result<String> {
        Ok(String::from_utf8_lossy(data).to_string())
    }
}
