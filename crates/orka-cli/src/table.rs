use comfy_table::{ContentArrangement, Table, presets};

/// Create a styled table ready to receive rows.
///
/// Automatically adjusts column widths to the terminal width.
/// Falls back to ASCII borders when the `NO_COLOR` environment variable is set.
pub fn make_table(headers: &[&str]) -> Table {
    let preset = if std::env::var_os("NO_COLOR").is_some() {
        presets::ASCII_FULL_CONDENSED
    } else {
        presets::UTF8_FULL_CONDENSED
    };
    let mut table = Table::new();
    table
        .load_preset(preset)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.to_vec());
    table
}
