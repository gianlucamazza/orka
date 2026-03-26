//! Inline terminal image rendering for media attachments.
//!
//! Uses [`viuer`] to auto-detect the best protocol available in the current
//! terminal: Kitty graphics, iTerm2, Sixel, or Unicode half-block fallback.
//! Falls back to saving a temp file and printing the path when the image
//! cannot be decoded or rendered.

use base64::Engine as _;

/// Render a base64-encoded image inline in the terminal, printing a header
/// with the caption above it. Returns the temp-file path used as fallback if
/// inline rendering is not available (e.g. non-interactive pipe).
///
/// When `multi` is provided (interactive chat mode), output is routed through
/// [`indicatif::MultiProgress::suspend`] so that viuer can write directly to
/// stdout without conflicting with the progress bar draw loop.
///
/// Prints nothing and returns `None` on a base64 decode error.
pub fn render_media(
    mime_type: &str,
    data_base64: &str,
    caption: Option<&str>,
    multi: Option<&indicatif::MultiProgress>,
) -> Option<String> {
    let bytes = match base64::engine::general_purpose::STANDARD.decode(data_base64) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[media] base64 decode error: {e}");
            return None;
        }
    };

    let label = caption.unwrap_or("Image");

    // Try to load with the `image` crate so viuer can display it.
    let img = match image::load_from_memory(&bytes) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("[media] failed to decode image: {e}");
            return save_fallback(&bytes, mime_type, label);
        }
    };

    // viuer config: let it pick the best protocol automatically.
    // Limit width to 80 columns so it fits comfortably in most terminals.
    let conf = viuer::Config {
        absolute_offset: false,
        width: Some(80),
        height: None,
        ..Default::default()
    };

    // When indicatif's MultiProgress is active, suspend it so viuer can write
    // directly to stdout without conflicting with the progress bar draw loop.
    let print_fn = || {
        println!("\n  {label}");
        viuer::print(&img, &conf)
    };
    let result = match multi {
        Some(m) => m.suspend(print_fn),
        None => print_fn(),
    };

    match result {
        Ok(_) => {
            println!(); // blank line after image
            None // rendered inline — no fallback file needed
        }
        Err(_) => {
            // viuer failed (e.g. piped stdout, dumb terminal) — save file instead.
            save_fallback(&bytes, mime_type, label)
        }
    }
}

fn save_fallback(bytes: &[u8], mime_type: &str, label: &str) -> Option<String> {
    let ext = if mime_type.contains("png") {
        "png"
    } else if mime_type.contains("jpeg") || mime_type.contains("jpg") {
        "jpg"
    } else {
        "bin"
    };
    let filename = format!("orka-{}.{ext}", uuid::Uuid::new_v4().simple());
    let path = std::env::temp_dir().join(&filename);
    match std::fs::write(&path, bytes) {
        Ok(()) => {
            let path_str = path.display().to_string();
            println!("  [{label}] saved: {path_str}");
            Some(path_str)
        }
        Err(e) => {
            eprintln!("  [{label}] failed to save: {e}");
            None
        }
    }
}
