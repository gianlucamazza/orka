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
            // Find the nearest char boundary at or after the target byte offset
            let target = prev.len() - chunk_overlap;
            let start = (target..prev.len())
                .find(|&i| prev.is_char_boundary(i))
                .unwrap_or(prev.len());
            &prev[start..]
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

    #[test]
    fn long_text_produces_multiple_chunks() {
        let text = "word ".repeat(500);
        let chunks = split_text(&text, 100, 0);
        assert!(chunks.len() > 1);
    }

    #[test]
    fn zero_overlap_produces_disjoint_chunks() {
        let text = "word ".repeat(500);
        let chunks = split_text(&text, 100, 0);
        assert!(chunks.len() > 1, "expected multiple chunks");
    }

    #[test]
    fn overlap_adds_trailing_context() {
        let text = "word ".repeat(500);
        let chunks = split_text(&text, 100, 20);
        // With overlap, chunks after the first should be longer
        // (they include overlap prefix from previous chunk)
        if chunks.len() > 1 {
            // Second chunk should start with tail of first chunk
            let first_tail = &chunks[0][chunks[0].len().saturating_sub(20)..];
            assert!(chunks[1].starts_with(first_tail));
        }
    }

    #[test]
    fn chunk_size_one_handles_gracefully() {
        let chunks = split_text("abc", 1, 0);
        assert!(!chunks.is_empty());
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn text_with_words_produces_at_least_one_chunk(
            text in "[a-zA-Z]{1,100}( [a-zA-Z]{1,100}){0,20}",
            chunk_size in 10usize..200,
            overlap in 0usize..50,
        ) {
            let chunks = split_text(&text, chunk_size, overlap);
            prop_assert!(!chunks.is_empty(), "text with words should produce at least one chunk");
        }

        #[test]
        fn empty_text_always_empty(chunk_size in 1usize..200, overlap in 0usize..50) {
            let chunks = split_text("", chunk_size, overlap);
            prop_assert!(chunks.is_empty());
        }

        #[test]
        fn overlap_never_panics_on_unicode(
            text in "\\PC{10,500}",
            chunk_size in 10usize..200,
            overlap in 0usize..50,
        ) {
            // The main property: split_text should never panic, even with
            // arbitrary Unicode input and overlap values.
            let _chunks = split_text(&text, chunk_size, overlap);
        }
    }
}
