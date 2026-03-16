use text_splitter::TextSplitter;

/// Split text into overlapping chunks.
pub fn split_text(text: &str, chunk_size: usize, chunk_overlap: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    let effective_size = chunk_size.max(1);
    let splitter = TextSplitter::new(effective_size);

    let chunks: Vec<String> = splitter.chunks(text).map(|s| s.to_string()).collect();

    if chunk_overlap == 0 || chunks.len() <= 1 {
        return chunks;
    }

    // Add overlap by including trailing context from previous chunk
    let mut overlapped = Vec::with_capacity(chunks.len());
    overlapped.push(chunks[0].clone());

    for i in 1..chunks.len() {
        let prev = &chunks[i - 1];
        let overlap_text = if prev.len() > chunk_overlap {
            &prev[prev.len() - chunk_overlap..]
        } else {
            prev.as_str()
        };
        overlapped.push(format!("{overlap_text}{}", chunks[i]));
    }

    overlapped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_returns_empty() {
        assert!(split_text("", 100, 20).is_empty());
    }

    #[test]
    fn short_text_returns_single_chunk() {
        let chunks = split_text("hello world", 1000, 200);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "hello world");
    }
}
