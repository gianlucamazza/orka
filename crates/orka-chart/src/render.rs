//! PNG chart renderer built on [`plotters`].

use plotters::{
    backend::BitMapBackend,
    chart::ChartBuilder,
    drawing::IntoDrawingArea as _,
    element::Circle,
    series::{AreaSeries, LineSeries},
    style::{Color as _, FontStyle, IntoFont as _, RGBColor, WHITE},
};

use crate::{
    Error,
    types::{ChartData, ChartSpec, ChartType, Series},
};

// ---------------------------------------------------------------------------
// Error conversion helpers
// ---------------------------------------------------------------------------

/// Convert any Display-able error into [`Error::Plotters`].
fn pe<E: std::fmt::Display>(e: E) -> Error {
    Error::Plotters(e.to_string())
}

// ---------------------------------------------------------------------------
// Default colour palette
// ---------------------------------------------------------------------------

const PALETTE: &[RGBColor] = &[
    RGBColor(66, 133, 244), // Google Blue
    RGBColor(219, 68, 55),  // Google Red
    RGBColor(244, 180, 0),  // Google Yellow
    RGBColor(15, 157, 88),  // Google Green
    RGBColor(171, 71, 188), // Purple
    RGBColor(0, 172, 193),  // Cyan
    RGBColor(255, 112, 67), // Deep Orange
    RGBColor(124, 179, 66), // Light Green
];

/// Parse a hex colour string such as `"#4CAF50"` or `"4CAF50"`.
fn parse_hex_color(s: &str) -> Option<RGBColor> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(RGBColor(r, g, b))
}

fn series_color(series: &Series, index: usize) -> RGBColor {
    series
        .color
        .as_deref()
        .and_then(parse_hex_color)
        .unwrap_or_else(|| PALETTE[index % PALETTE.len()])
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render a [`ChartSpec`] to an in-memory PNG buffer.
pub fn render_chart(spec: &ChartSpec) -> Result<Vec<u8>, Error> {
    match &spec.chart_type {
        ChartType::Bar | ChartType::StackedBar => render_bar(spec),
        ChartType::Line | ChartType::Combo => render_line(spec),
        ChartType::Area => render_area(spec),
        ChartType::Pie => render_pie(spec),
        ChartType::Scatter => render_scatter(spec),
        ChartType::Histogram => render_histogram(spec),
    }
}

// ---------------------------------------------------------------------------
// Bar chart
// ---------------------------------------------------------------------------

fn render_bar(spec: &ChartSpec) -> Result<Vec<u8>, Error> {
    let (w, h) = (spec.width, spec.height);
    let mut buf = vec![0u8; (w * h * 3) as usize];
    {
        let root = BitMapBackend::with_buffer(&mut buf, (w, h)).into_drawing_area();
        root.fill(&WHITE).map_err(pe)?;

        let data = &spec.data;
        let n_groups = data.labels.len().max(
            data.series
                .iter()
                .map(|s| s.values.len())
                .max()
                .unwrap_or(0),
        );
        if n_groups == 0 {
            return Err(Error::Render("no data points".into()));
        }
        let n_series = data.series.len().max(1);
        let max_val = data
            .series
            .iter()
            .flat_map(|s| s.values.iter().copied())
            .fold(0.0_f64, f64::max)
            * 1.1;
        let max_val = if max_val == 0.0 { 1.0 } else { max_val };

        let mut chart = ChartBuilder::on(&root)
            .caption(
                spec.title.as_deref().unwrap_or(""),
                ("sans-serif", 20).into_font(),
            )
            .margin(20)
            .x_label_area_size(40)
            .y_label_area_size(60)
            .build_cartesian_2d(0u32..(n_groups as u32 * n_series as u32), 0.0..max_val)
            .map_err(pe)?;

        chart
            .configure_mesh()
            .x_labels(n_groups)
            .x_label_formatter(&|v| {
                let group = (*v as usize) / n_series;
                data.labels
                    .get(group)
                    .cloned()
                    .unwrap_or_else(|| group.to_string())
            })
            .y_label_formatter(&|v| format!("{v:.0}"))
            .x_desc(spec.x_label.as_deref().unwrap_or(""))
            .y_desc(spec.y_label.as_deref().unwrap_or(""))
            .draw()
            .map_err(pe)?;

        for (si, series) in data.series.iter().enumerate() {
            let color = series_color(series, si);
            let bar_data: Vec<(u32, f64)> = series
                .values
                .iter()
                .enumerate()
                .map(|(i, &v)| ((i * n_series + si) as u32, v))
                .collect();
            chart
                .draw_series(bar_data.iter().map(|&(x, y)| {
                    plotters::element::Rectangle::new([(x, 0.0), (x + 1, y)], color.filled())
                }))
                .map_err(pe)?;
        }

        root.present().map_err(pe)?;
    }
    encode_rgb_to_png(buf, w, h)
}

// ---------------------------------------------------------------------------
// Line chart
// ---------------------------------------------------------------------------

fn render_line(spec: &ChartSpec) -> Result<Vec<u8>, Error> {
    let (w, h) = (spec.width, spec.height);
    let mut buf = vec![0u8; (w * h * 3) as usize];
    {
        let root = BitMapBackend::with_buffer(&mut buf, (w, h)).into_drawing_area();
        root.fill(&WHITE).map_err(pe)?;

        let data = &spec.data;
        let n_points = max_series_len(data);
        if n_points == 0 {
            return Err(Error::Render("no data points".into()));
        }
        let (y_min, y_max) = y_range(data);

        let mut chart = ChartBuilder::on(&root)
            .caption(
                spec.title.as_deref().unwrap_or(""),
                ("sans-serif", 20).into_font(),
            )
            .margin(20)
            .x_label_area_size(40)
            .y_label_area_size(60)
            .build_cartesian_2d(0.0..(n_points as f64 - 1.0), y_min..y_max)
            .map_err(pe)?;

        chart
            .configure_mesh()
            .x_label_formatter(&|v| {
                let i = *v as usize;
                data.labels.get(i).cloned().unwrap_or_else(|| i.to_string())
            })
            .x_desc(spec.x_label.as_deref().unwrap_or(""))
            .y_desc(spec.y_label.as_deref().unwrap_or(""))
            .draw()
            .map_err(pe)?;

        for (si, series) in data.series.iter().enumerate() {
            let color = series_color(series, si);
            let points: Vec<(f64, f64)> = series
                .values
                .iter()
                .enumerate()
                .map(|(i, &v)| (i as f64, v))
                .collect();
            chart
                .draw_series(LineSeries::new(points, color.stroke_width(2)))
                .map_err(pe)?
                .label(&series.name)
                .legend(move |(x, y)| {
                    plotters::element::PathElement::new(vec![(x, y), (x + 20, y)], color)
                });
        }

        chart
            .configure_series_labels()
            .border_style(plotters::style::BLACK)
            .draw()
            .map_err(pe)?;

        root.present().map_err(pe)?;
    }
    encode_rgb_to_png(buf, w, h)
}

// ---------------------------------------------------------------------------
// Area chart
// ---------------------------------------------------------------------------

fn render_area(spec: &ChartSpec) -> Result<Vec<u8>, Error> {
    let (w, h) = (spec.width, spec.height);
    let mut buf = vec![0u8; (w * h * 3) as usize];
    {
        let root = BitMapBackend::with_buffer(&mut buf, (w, h)).into_drawing_area();
        root.fill(&WHITE).map_err(pe)?;

        let data = &spec.data;
        let n_points = max_series_len(data);
        if n_points == 0 {
            return Err(Error::Render("no data points".into()));
        }
        let (_, y_max) = y_range(data);

        let mut chart = ChartBuilder::on(&root)
            .caption(
                spec.title.as_deref().unwrap_or(""),
                ("sans-serif", 20).into_font(),
            )
            .margin(20)
            .x_label_area_size(40)
            .y_label_area_size(60)
            .build_cartesian_2d(0.0..(n_points as f64 - 1.0), 0.0..y_max)
            .map_err(pe)?;

        chart
            .configure_mesh()
            .x_label_formatter(&|v| {
                let i = *v as usize;
                data.labels.get(i).cloned().unwrap_or_else(|| i.to_string())
            })
            .x_desc(spec.x_label.as_deref().unwrap_or(""))
            .y_desc(spec.y_label.as_deref().unwrap_or(""))
            .draw()
            .map_err(pe)?;

        for (si, series) in data.series.iter().enumerate() {
            let color = series_color(series, si);
            let points: Vec<(f64, f64)> = series
                .values
                .iter()
                .enumerate()
                .map(|(i, &v)| (i as f64, v))
                .collect();
            chart
                .draw_series(
                    AreaSeries::new(points, 0.0, color.mix(0.3))
                        .border_style(color.stroke_width(2)),
                )
                .map_err(pe)?;
        }

        root.present().map_err(pe)?;
    }
    encode_rgb_to_png(buf, w, h)
}

// ---------------------------------------------------------------------------
// Pie chart
// ---------------------------------------------------------------------------

fn render_pie(spec: &ChartSpec) -> Result<Vec<u8>, Error> {
    let (w, h) = (spec.width, spec.height);
    let mut buf = vec![0u8; (w * h * 3) as usize];
    {
        let root = BitMapBackend::with_buffer(&mut buf, (w, h)).into_drawing_area();
        root.fill(&WHITE).map_err(pe)?;

        // Use first series for pie data
        let data = &spec.data;
        let series = data
            .series
            .first()
            .ok_or_else(|| Error::Render("no series".into()))?;
        let total: f64 = series.values.iter().sum();
        if total == 0.0 {
            return Err(Error::Render("all values are zero".into()));
        }

        let cx = (w / 2) as i32;
        let cy = (h / 2) as i32;
        let r = ((w.min(h) / 2) as i32) - 40;

        // Title
        if let Some(title) = &spec.title {
            root.draw(&plotters::element::Text::new(
                title.as_str(),
                (cx, 20),
                ("sans-serif", 20)
                    .into_font()
                    .style(FontStyle::Bold)
                    .color(&plotters::style::BLACK),
            ))
            .map_err(pe)?;
        }

        let mut start_angle: f64 = -std::f64::consts::FRAC_PI_2;
        for (i, (&value, label)) in series
            .values
            .iter()
            .zip(data.labels.iter().chain(std::iter::repeat(&String::new())))
            .enumerate()
        {
            let fraction = value / total;
            let sweep = fraction * 2.0 * std::f64::consts::PI;
            let end_angle = start_angle + sweep;
            let color = series_color(series, i);

            // Draw wedge as a polygon
            let steps = ((sweep * 180.0 / std::f64::consts::PI) as usize).max(2);
            let mut points: Vec<(i32, i32)> = vec![(cx, cy)];
            let rf = f64::from(r);
            for step in 0..=steps {
                let angle = start_angle + (step as f64 / steps as f64) * sweep;
                points.push((
                    cx + (rf * angle.cos()) as i32,
                    cy + (rf * angle.sin()) as i32,
                ));
            }
            root.draw(&plotters::element::Polygon::new(points, color.filled()))
                .map_err(pe)?;

            // Label at mid-angle
            if !label.is_empty() || fraction > 0.05 {
                let mid = start_angle + sweep / 2.0;
                let lx = cx + ((rf * 0.7) * mid.cos()) as i32;
                let ly = cy + ((rf * 0.7) * mid.sin()) as i32;
                let pct = format!("{:.0}%", fraction * 100.0);
                root.draw(&plotters::element::Text::new(
                    pct.as_str(),
                    (lx - 15, ly - 8),
                    ("sans-serif", 12)
                        .into_font()
                        .color(&plotters::style::WHITE),
                ))
                .map_err(pe)?;
            }

            start_angle = end_angle;
        }

        root.present().map_err(pe)?;
    }
    encode_rgb_to_png(buf, w, h)
}

// ---------------------------------------------------------------------------
// Scatter plot
// ---------------------------------------------------------------------------

fn render_scatter(spec: &ChartSpec) -> Result<Vec<u8>, Error> {
    let (w, h) = (spec.width, spec.height);
    let mut buf = vec![0u8; (w * h * 3) as usize];
    {
        let root = BitMapBackend::with_buffer(&mut buf, (w, h)).into_drawing_area();
        root.fill(&WHITE).map_err(pe)?;

        let data = &spec.data;
        let n_points = max_series_len(data);
        if n_points == 0 {
            return Err(Error::Render("no data points".into()));
        }
        let (y_min, y_max) = y_range(data);

        let mut chart = ChartBuilder::on(&root)
            .caption(
                spec.title.as_deref().unwrap_or(""),
                ("sans-serif", 20).into_font(),
            )
            .margin(20)
            .x_label_area_size(40)
            .y_label_area_size(60)
            .build_cartesian_2d(0.0..(n_points as f64 - 1.0), y_min..y_max)
            .map_err(pe)?;

        chart
            .configure_mesh()
            .x_label_formatter(&|v| {
                let i = *v as usize;
                data.labels.get(i).cloned().unwrap_or_else(|| i.to_string())
            })
            .x_desc(spec.x_label.as_deref().unwrap_or(""))
            .y_desc(spec.y_label.as_deref().unwrap_or(""))
            .draw()
            .map_err(pe)?;

        for (si, series) in data.series.iter().enumerate() {
            let color = series_color(series, si);
            let points: Vec<(f64, f64)> = series
                .values
                .iter()
                .enumerate()
                .map(|(i, &v)| (i as f64, v))
                .collect();
            chart
                .draw_series(
                    points
                        .iter()
                        .map(|&(x, y)| Circle::new((x, y), 4, color.filled())),
                )
                .map_err(pe)?;
        }

        root.present().map_err(pe)?;
    }
    encode_rgb_to_png(buf, w, h)
}

// ---------------------------------------------------------------------------
// Histogram
// ---------------------------------------------------------------------------

fn render_histogram(spec: &ChartSpec) -> Result<Vec<u8>, Error> {
    // Histogram reuses bar chart rendering logic
    render_bar(spec)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn max_series_len(data: &ChartData) -> usize {
    data.series
        .iter()
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0)
}

fn y_range(data: &ChartData) -> (f64, f64) {
    let min = data
        .series
        .iter()
        .flat_map(|s| s.values.iter().copied())
        .fold(f64::INFINITY, f64::min);
    let max = data
        .series
        .iter()
        .flat_map(|s| s.values.iter().copied())
        .fold(f64::NEG_INFINITY, f64::max);
    let min = if min.is_infinite() { 0.0 } else { min };
    let max = if max.is_infinite() || (max - min).abs() < f64::EPSILON {
        min + 1.0
    } else {
        max * 1.1
    };
    // Always include 0 for bar-like charts
    (min.min(0.0), max)
}

/// Encode a raw RGB24 buffer produced by `plotters::BitMapBackend` into PNG
/// bytes.
fn encode_rgb_to_png(rgb: Vec<u8>, width: u32, height: u32) -> Result<Vec<u8>, Error> {
    let img: image::RgbImage = image::ImageBuffer::from_raw(width, height, rgb)
        .ok_or_else(|| Error::Render("image buffer dimensions mismatch".into()))?;
    let mut png_bytes: Vec<u8> = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_bytes),
        image::ImageOutputFormat::Png,
    )
    .map_err(|e| Error::Render(format!("PNG encode failed: {e}")))?;
    Ok(png_bytes)
}
