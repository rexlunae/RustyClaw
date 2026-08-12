//! Chart rendering, for answers that are better seen than read.
//!
//! Emits a self-contained SVG. No plotting crate and no headless browser: a
//! chart is axes, ticks and a handful of shapes, and a dependency that pulls
//! in a rendering stack to draw a bar costs more than it saves. SVG also
//! survives the trip to every client the gateway talks to — the desktop
//! renders it inline, a messenger can attach it, and it stays readable in a
//! terminal-shaped world because it is text.
//!
//! Colours come from a fixed palette chosen to stay distinguishable in both
//! light and dark themes, and the chart paints its own background rather than
//! inheriting the viewer's, so a dark-theme client does not get dark text on
//! a transparent ground.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

use super::error::{ToolError, ToolResult};

/// Plot area geometry. Margins leave room for the title, axis labels and the
/// legend; the plot itself is what remains.
const WIDTH: f64 = 720.0;
const HEIGHT: f64 = 440.0;
const MARGIN_LEFT: f64 = 70.0;
const MARGIN_RIGHT: f64 = 24.0;
const MARGIN_TOP: f64 = 48.0;
const MARGIN_BOTTOM: f64 = 64.0;

/// Series colours, in assignment order.
///
/// Picked for contrast against both a near-white and a near-black ground, so
/// one palette serves both themes. Wraps if a caller supplies more series
/// than colours — repeating a colour is worse than crashing is worse than
/// inventing an unreadable one.
const PALETTE: &[&str] = &[
    "#3b7dd8", "#e0662b", "#2e9e6b", "#b5522f", "#8250c4", "#c04a86", "#7a7f87", "#b8992c",
];

const INK: &str = "#1c1f24";
const MUTED: &str = "#6b7280";
const GRID: &str = "#d8dce2";
const GROUND: &str = "#ffffff";

/// One named series of values.
struct Series {
    label: String,
    values: Vec<f64>,
}

/// Render a chart to SVG and write it into the workspace.
///
/// `chart_type` selects the shape; `series` carries the data. The result is a
/// path, not the SVG itself: charts run to tens of kilobytes and a model does
/// not need the markup echoed back into its context to know the file exists.
pub fn exec_chart(args: &Value, workspace_dir: &Path) -> ToolResult {
    let chart_type = args
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("bar")
        .to_ascii_lowercase();

    let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let x_label = args.get("x_label").and_then(|v| v.as_str()).unwrap_or("");
    let y_label = args.get("y_label").and_then(|v| v.as_str()).unwrap_or("");

    let categories = parse_categories(args);
    let series = parse_series(args)?;

    // Checked before rendering rather than producing an empty chart: an SVG
    // with no marks looks like a rendering failure, and the caller cannot tell
    // it apart from one.
    if series.is_empty() {
        return Err(ToolError::msg(
            "chart needs at least one series — pass `series` as a list of \
             {name, values} objects, or `values` for a single unnamed series",
        ));
    }
    // Every series, not merely one of them: an empty series contributes
    // nothing but still counts toward the "N series" in the success message,
    // and for a pie it is the one that gets drawn.
    if series.iter().any(|s| s.values.is_empty()) {
        return Err(ToolError::msg(
            "a series has no values; a chart of nothing is not a chart",
        ));
    }

    let svg = match chart_type.as_str() {
        "bar" => render_bar(title, x_label, y_label, &categories, &series),
        "line" => render_line(title, x_label, y_label, &categories, &series, false),
        "scatter" => render_line(title, x_label, y_label, &categories, &series, true),
        "pie" => {
            // A pie shows one set of proportions. Drawing `series[0]` and
            // discarding the rest loses data with no error, and the success
            // message would still have claimed every series was drawn.
            if series.len() > 1 {
                return Err(ToolError::msg(format!(
                    "a pie chart shows one set of proportions, but {} series were \
                     given; pass a single `values` list, or use type \"bar\" to \
                     compare series",
                    series.len()
                )));
            }
            render_pie(title, &categories, &series[0])
        }
        other => {
            return Err(ToolError::msg(format!(
                "unknown chart type {other:?}; expected one of bar, line, scatter, pie"
            )));
        }
    };

    let rel = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("chart.svg");
    let out = resolve_output(workspace_dir, rel)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ToolError::context("Could not create the chart's directory", e))?;
    }
    std::fs::write(&out, svg.as_bytes())
        .map_err(|e| ToolError::context("Could not write the chart", e))?;

    Ok(format!(
        "Wrote a {chart_type} chart with {} series to {}",
        series.len(),
        out.display()
    ))
}

/// Keep the output inside the workspace.
///
/// The same containment every other file-writing tool applies: a chart is not
/// a reason to gain write access to `/etc`, and `..` in a model-supplied path
/// is far more likely to be a mistake than an intention.
fn resolve_output(workspace_dir: &Path, rel: &str) -> ToolResult<std::path::PathBuf> {
    let candidate = if Path::new(rel).is_absolute() {
        std::path::PathBuf::from(rel)
    } else {
        workspace_dir.join(rel)
    };
    // `..` is refused outright rather than normalised away. A chart filename
    // has no legitimate use for it, and leaving it in is what makes the
    // containment check foolable: `new/../../out.svg` has a parent that cannot
    // be canonicalised (because `new` does not exist yet), so any
    // resolve-then-compare falls back to the literal path, passes a prefix
    // check, and then `create_dir_all` plus the write resolve the `..` at the
    // filesystem level and land outside.
    if candidate
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ToolError::msg(
            "chart path must not contain `..` — give a path inside the workspace",
        ));
    }

    let root = workspace_dir
        .canonicalize()
        .unwrap_or_else(|_| workspace_dir.to_path_buf());

    // Canonicalise the deepest ancestor that already exists. Everything below
    // it is about to be created *inside whatever that resolves to*, so that is
    // the thing containment has to be judged on. Checking only the immediate
    // parent misses `link/sub/out.svg`, where `link` is a symlink out of the
    // workspace and `sub` does not exist yet — the parent cannot be resolved,
    // and the symlink one level up never gets looked at.
    let mut existing = candidate.as_path();
    while !existing.exists() {
        match existing.parent() {
            Some(parent) if parent != existing => existing = parent,
            _ => break,
        }
    }
    let anchor = existing
        .canonicalize()
        .map_err(|e| ToolError::context("Could not resolve the chart path", e))?;
    if !anchor.starts_with(&root) {
        return Err(ToolError::msg(format!(
            "chart path {} escapes the workspace",
            candidate.display()
        )));
    }

    // Rebuild from the resolved anchor so the path handed to the writer is the
    // one that was actually checked.
    let rest = candidate.strip_prefix(existing).unwrap_or(Path::new(""));
    Ok(anchor.join(rest))
}

/// Category labels for the x axis, when the caller supplied them.
fn parse_categories(args: &Value) -> Vec<String> {
    args.get("categories")
        .or_else(|| args.get("labels"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Accepts either `series: [{name, values}]` or a bare `values: [..]`.
///
/// Both shapes exist because a model asked for "a chart of these numbers"
/// should not have to wrap one list in two layers of structure, while
/// multi-series charts genuinely need the names.
fn parse_series(args: &Value) -> ToolResult<Vec<Series>> {
    if let Some(list) = args.get("series").and_then(|v| v.as_array()) {
        let mut out = Vec::with_capacity(list.len());
        for (i, entry) in list.iter().enumerate() {
            let label = entry
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("Series {}", i + 1));
            let values = numbers(entry.get("values"), &label)?;
            out.push(Series { label, values });
        }
        return Ok(out);
    }
    if let Some(values) = args.get("values") {
        return Ok(vec![Series {
            label: args
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Series 1")
                .to_string(),
            values: numbers(Some(values), "values")?,
        }]);
    }
    Ok(Vec::new())
}

/// A list of numbers, refusing anything that is not one.
///
/// Silently dropping a non-numeric entry would shift every later point one
/// place to the left against its category label, which is a wrong chart
/// rather than a missing one.
fn numbers(v: Option<&Value>, whose: &str) -> ToolResult<Vec<f64>> {
    let Some(Value::Array(items)) = v else {
        return Err(ToolError::msg(format!("{whose} must be a list of numbers")));
    };
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            item.as_f64().ok_or_else(|| {
                ToolError::msg(format!("{whose}[{i}] is {item}, which is not a number"))
            })
        })
        .collect()
}

/// XML-escape text destined for an SVG text node or attribute.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Trim a float to a short, stable decimal form.
fn n(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The value range to plot, always including zero for bar charts so a bar's
/// length stays proportional to its value.
fn value_range(series: &[Series], include_zero: bool) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for s in series {
        for &v in &s.values {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return (0.0, 1.0);
    }
    if include_zero {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    // A flat series has no range to scale against; give it one so the line
    // lands mid-plot instead of dividing by zero.
    if (hi - lo).abs() < f64::EPSILON {
        return (lo - 1.0, hi + 1.0);
    }
    (lo, hi)
}

/// Document header, background and title.
fn open_svg(title: &str) -> String {
    let mut s = String::new();
    _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{HEIGHT}" viewBox="0 0 {WIDTH} {HEIGHT}" font-family="system-ui, -apple-system, Segoe UI, sans-serif">"#
    );
    // An explicit ground: without it the chart borrows the viewer's
    // background, and dark-theme clients get near-black text on near-black.
    _ = write!(
        s,
        r#"<rect width="{WIDTH}" height="{HEIGHT}" fill="{GROUND}"/>"#
    );
    if !title.is_empty() {
        _ = write!(
            s,
            r#"<text x="{}" y="28" text-anchor="middle" font-size="17" font-weight="600" fill="{INK}">{}</text>"#,
            WIDTH / 2.0,
            esc(title)
        );
    }
    s
}

/// Axes, gridlines, tick labels and axis titles.
fn axes(svg: &mut String, lo: f64, hi: f64, x_label: &str, y_label: &str) {
    let plot_h = HEIGHT - MARGIN_TOP - MARGIN_BOTTOM;
    let plot_w = WIDTH - MARGIN_LEFT - MARGIN_RIGHT;
    const TICKS: usize = 5;

    for i in 0..=TICKS {
        let frac = i as f64 / TICKS as f64;
        let y = MARGIN_TOP + plot_h * (1.0 - frac);
        let value = lo + (hi - lo) * frac;
        _ = write!(
            svg,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{GRID}" stroke-width="1"/>"#,
            n(MARGIN_LEFT),
            n(y),
            n(MARGIN_LEFT + plot_w),
            n(y)
        );
        _ = write!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="end" font-size="11" fill="{MUTED}">{}</text>"#,
            n(MARGIN_LEFT - 8.0),
            n(y + 4.0),
            n(value)
        );
    }
    _ = write!(
        svg,
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{INK}" stroke-width="1.5"/>"#,
        n(MARGIN_LEFT),
        n(MARGIN_TOP),
        n(MARGIN_LEFT),
        n(MARGIN_TOP + plot_h)
    );
    _ = write!(
        svg,
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{INK}" stroke-width="1.5"/>"#,
        n(MARGIN_LEFT),
        n(MARGIN_TOP + plot_h),
        n(MARGIN_LEFT + plot_w),
        n(MARGIN_TOP + plot_h)
    );
    if !x_label.is_empty() {
        _ = write!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="middle" font-size="12" fill="{MUTED}">{}</text>"#,
            n(MARGIN_LEFT + plot_w / 2.0),
            n(HEIGHT - 14.0),
            esc(x_label)
        );
    }
    if !y_label.is_empty() {
        _ = write!(
            svg,
            r#"<text transform="translate(16,{}) rotate(-90)" text-anchor="middle" font-size="12" fill="{MUTED}">{}</text>"#,
            n(MARGIN_TOP + plot_h / 2.0),
            esc(y_label)
        );
    }
}

/// Series legend, drawn along the bottom.
fn legend(svg: &mut String, series: &[Series]) {
    if series.len() < 2 {
        return;
    }
    let mut x = MARGIN_LEFT;
    let y = HEIGHT - 34.0;
    for (i, s) in series.iter().enumerate() {
        let colour = PALETTE[i % PALETTE.len()];
        _ = write!(
            svg,
            r#"<rect x="{}" y="{}" width="10" height="10" fill="{colour}"/>"#,
            n(x),
            n(y - 9.0)
        );
        _ = write!(
            svg,
            r#"<text x="{}" y="{}" font-size="11" fill="{INK}">{}</text>"#,
            n(x + 15.0),
            n(y),
            esc(&s.label)
        );
        x += 15.0 + 8.0 * s.label.chars().count() as f64 + 22.0;
    }
}

/// The category label under slot `i`, or its 1-based index if unnamed.
fn category(categories: &[String], i: usize) -> String {
    categories
        .get(i)
        .cloned()
        .unwrap_or_else(|| (i + 1).to_string())
}

fn render_bar(
    title: &str,
    x_label: &str,
    y_label: &str,
    categories: &[String],
    series: &[Series],
) -> String {
    let mut svg = open_svg(title);
    let (lo, hi) = value_range(series, true);
    axes(&mut svg, lo, hi, x_label, y_label);

    let plot_h = HEIGHT - MARGIN_TOP - MARGIN_BOTTOM;
    let plot_w = WIDTH - MARGIN_LEFT - MARGIN_RIGHT;
    let slots = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if slots == 0 {
        svg.push_str("</svg>");
        return svg;
    }
    let slot_w = plot_w / slots as f64;
    let bar_w = (slot_w * 0.72) / series.len() as f64;
    let zero_y = MARGIN_TOP + plot_h * (1.0 - (0.0 - lo) / (hi - lo));

    for (si, s) in series.iter().enumerate() {
        let colour = PALETTE[si % PALETTE.len()];
        for (i, &v) in s.values.iter().enumerate() {
            let y = MARGIN_TOP + plot_h * (1.0 - (v - lo) / (hi - lo));
            let x = MARGIN_LEFT + slot_w * i as f64 + slot_w * 0.14 + bar_w * si as f64;
            let (top, height) = if v >= 0.0 {
                (y, zero_y - y)
            } else {
                (zero_y, y - zero_y)
            };
            _ = write!(
                svg,
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{colour}"><title>{}: {}</title></rect>"#,
                n(x),
                n(top),
                n(bar_w),
                n(height.max(0.0)),
                esc(&s.label),
                n(v)
            );
        }
    }
    for i in 0..slots {
        _ = write!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="middle" font-size="11" fill="{MUTED}">{}</text>"#,
            n(MARGIN_LEFT + slot_w * (i as f64 + 0.5)),
            n(MARGIN_TOP + plot_h + 16.0),
            esc(&category(categories, i))
        );
    }
    legend(&mut svg, series);
    svg.push_str("</svg>");
    svg
}

fn render_line(
    title: &str,
    x_label: &str,
    y_label: &str,
    categories: &[String],
    series: &[Series],
    points_only: bool,
) -> String {
    let mut svg = open_svg(title);
    let (lo, hi) = value_range(series, false);
    axes(&mut svg, lo, hi, x_label, y_label);

    let plot_h = HEIGHT - MARGIN_TOP - MARGIN_BOTTOM;
    let plot_w = WIDTH - MARGIN_LEFT - MARGIN_RIGHT;
    let slots = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if slots == 0 {
        svg.push_str("</svg>");
        return svg;
    }
    // A single point has no span to divide; centre it rather than dividing by
    // zero and placing it at infinity.
    let step = if slots > 1 {
        plot_w / (slots - 1) as f64
    } else {
        0.0
    };
    let x_at = |i: usize| {
        if slots > 1 {
            MARGIN_LEFT + step * i as f64
        } else {
            MARGIN_LEFT + plot_w / 2.0
        }
    };

    for (si, s) in series.iter().enumerate() {
        let colour = PALETTE[si % PALETTE.len()];
        let pts: Vec<(f64, f64)> = s
            .values
            .iter()
            .enumerate()
            .map(|(i, &v)| (x_at(i), MARGIN_TOP + plot_h * (1.0 - (v - lo) / (hi - lo))))
            .collect();
        if !points_only && pts.len() > 1 {
            let path: Vec<String> = pts
                .iter()
                .map(|(x, y)| format!("{},{}", n(*x), n(*y)))
                .collect();
            _ = write!(
                svg,
                r#"<polyline points="{}" fill="none" stroke="{colour}" stroke-width="2" stroke-linejoin="round"/>"#,
                path.join(" ")
            );
        }
        for ((x, y), v) in pts.iter().zip(&s.values) {
            _ = write!(
                svg,
                r#"<circle cx="{}" cy="{}" r="3.5" fill="{colour}"><title>{}: {}</title></circle>"#,
                n(*x),
                n(*y),
                esc(&s.label),
                n(*v)
            );
        }
    }
    for i in 0..slots {
        _ = write!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="middle" font-size="11" fill="{MUTED}">{}</text>"#,
            n(x_at(i)),
            n(MARGIN_TOP + plot_h + 16.0),
            esc(&category(categories, i))
        );
    }
    legend(&mut svg, series);
    svg.push_str("</svg>");
    svg
}

fn render_pie(title: &str, categories: &[String], series: &Series) -> String {
    let mut svg = open_svg(title);
    // Negative slices have no meaning in a pie — a share of a whole cannot be
    // below nothing — so they are dropped rather than drawn inside out.
    let values: Vec<f64> = series.values.iter().map(|v| v.max(0.0)).collect();
    let total: f64 = values.iter().sum();
    let cx = WIDTH / 2.0 - 80.0;
    let cy = MARGIN_TOP + (HEIGHT - MARGIN_TOP - MARGIN_BOTTOM) / 2.0;
    let r = 140.0_f64;

    if total <= 0.0 {
        _ = write!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="middle" font-size="13" fill="{MUTED}">no positive values to plot</text>"#,
            n(WIDTH / 2.0),
            n(cy)
        );
        svg.push_str("</svg>");
        return svg;
    }

    let mut angle = -std::f64::consts::FRAC_PI_2;
    for (i, &v) in values.iter().enumerate() {
        if v <= 0.0 {
            continue;
        }
        let sweep = v / total * std::f64::consts::TAU;
        let end = angle + sweep;
        let colour = PALETTE[i % PALETTE.len()];
        let (x0, y0) = (cx + r * angle.cos(), cy + r * angle.sin());
        let (x1, y1) = (cx + r * end.cos(), cy + r * end.sin());
        // A slice larger than half the circle needs the large-arc flag, or the
        // renderer takes the short way round and draws its complement.
        let large = if sweep > std::f64::consts::PI { 1 } else { 0 };
        let label = category(categories, i);
        // A slice covering the whole circle has an arc whose end point is its
        // start point, and the SVG spec says a renderer drops such a segment —
        // so a one-value pie drew nothing but its title. A circle has no start
        // and end to coincide.
        if sweep >= std::f64::consts::TAU - 1e-9 {
            _ = write!(
                svg,
                r#"<circle cx="{}" cy="{}" r="{r}" fill="{colour}"><title>{}: {} (100%)</title></circle>"#,
                n(cx),
                n(cy),
                esc(&label),
                n(v)
            );
            angle = end;
            continue;
        }
        _ = write!(
            svg,
            r#"<path d="M {} {} L {} {} A {r} {r} 0 {large} 1 {} {} Z" fill="{colour}"><title>{}: {} ({}%)</title></path>"#,
            n(cx),
            n(cy),
            n(x0),
            n(y0),
            n(x1),
            n(y1),
            esc(&label),
            n(v),
            n(v / total * 100.0)
        );
        angle = end;
    }

    // Legend to the right: pie slices are too narrow to label in place once
    // any of them is small.
    let mut ly = MARGIN_TOP + 10.0;
    for (i, &v) in values.iter().enumerate() {
        if v <= 0.0 {
            continue;
        }
        let colour = PALETTE[i % PALETTE.len()];
        _ = write!(
            svg,
            r#"<rect x="{}" y="{}" width="11" height="11" fill="{colour}"/>"#,
            n(WIDTH - 190.0),
            n(ly - 9.0)
        );
        _ = write!(
            svg,
            r#"<text x="{}" y="{}" font-size="12" fill="{INK}">{} ({}%)</text>"#,
            n(WIDTH - 173.0),
            n(ly),
            esc(&category(categories, i)),
            n(v / total * 100.0)
        );
        ly += 20.0;
    }
    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn render(args: Value) -> String {
        let dir = tempfile::tempdir().expect("temp dir");
        exec_chart(&args, dir.path()).expect("chart renders");
        std::fs::read_to_string(dir.path().join("chart.svg")).expect("chart written")
    }

    #[test]
    fn a_bar_chart_draws_one_rect_per_value() {
        let svg = render(json!({
            "type": "bar",
            "title": "Requests",
            "categories": ["mon", "tue", "wed"],
            "values": [3, 5, 4],
        }));
        // Three bars plus the background rect.
        assert_eq!(svg.matches("<rect").count(), 4, "{svg}");
        assert!(svg.contains("Requests"));
        assert!(svg.contains(">mon<"));
    }

    /// The whole point of the tool is a picture a human looks at, so the
    /// output has to be a real SVG document, not a fragment.
    #[test]
    fn the_output_is_a_self_contained_svg() {
        let svg = render(json!({"values": [1, 2]}));
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.ends_with("</svg>"));
        // An explicit ground, so a dark-theme viewer does not get dark text on
        // its own dark background.
        assert!(svg.contains(&format!("fill=\"{GROUND}\"")), "{svg}");
    }

    /// A label carrying `<` or `&` must not be able to close a tag or open a
    /// new one — the model supplies these strings.
    #[test]
    fn labels_are_escaped_rather_than_injected() {
        let svg = render(json!({
            "title": "a < b & c",
            "categories": ["<script>x</script>"],
            "values": [1],
        }));
        assert!(svg.contains("a &lt; b &amp; c"), "{svg}");
        assert!(!svg.contains("<script>"), "{svg}");
    }

    /// Dropping a non-numeric entry would shift every later value one place
    /// left against its category label: a wrong chart, not a missing one.
    #[test]
    fn a_non_numeric_value_is_refused_rather_than_skipped() {
        let dir = tempfile::tempdir().expect("temp dir");
        let err =
            exec_chart(&json!({"values": [1, "two", 3]}), dir.path()).expect_err("must be refused");
        assert!(format!("{err}").contains("not a number"), "{err}");
    }

    #[test]
    fn an_empty_chart_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(exec_chart(&json!({}), dir.path()).is_err());
        assert!(exec_chart(&json!({"values": []}), dir.path()).is_err());
    }

    /// A pie slice past the half-way mark needs the large-arc flag, or the
    /// renderer draws its complement and the chart silently lies.
    #[test]
    fn a_majority_pie_slice_sets_the_large_arc_flag() {
        let svg = render(json!({
            "type": "pie",
            "categories": ["most", "rest"],
            "values": [80, 20],
        }));
        assert!(svg.contains(" 1 1 "), "no large-arc flag set: {svg}");
        assert!(svg.contains("(80%)"), "{svg}");
    }

    /// A flat series has no range to scale against; it must not divide by zero
    /// and place every point at infinity.
    #[test]
    fn a_flat_series_still_renders() {
        let svg = render(json!({"type": "line", "values": [7, 7, 7]}));
        assert!(!svg.contains("NaN"), "{svg}");
        assert!(!svg.contains("inf"), "{svg}");
    }

    /// One point has no span to divide across.
    #[test]
    fn a_single_point_does_not_divide_by_zero() {
        let svg = render(json!({"type": "line", "values": [42]}));
        assert!(!svg.contains("NaN") && !svg.contains("inf"), "{svg}");
    }

    /// A slice covering the whole circle has an arc whose end point equals its
    /// start point, and the SVG spec says renderers drop such a segment — so a
    /// one-value pie drew nothing but its title. Nothing errored; the picture
    /// was simply blank.
    #[test]
    fn a_single_slice_pie_draws_a_circle_not_a_vanishing_arc() {
        let svg = render(json!({
            "type": "pie",
            "categories": ["everything"],
            "values": [5],
        }));
        assert!(svg.contains("<circle"), "the only slice vanished: {svg}");
        assert!(svg.contains("(100%)"), "{svg}");
    }

    /// The same when several values are given but only one is positive: the
    /// others are dropped, so the survivor still spans the full circle.
    #[test]
    fn one_positive_value_among_many_still_draws_a_circle() {
        let svg = render(json!({
            "type": "pie",
            "categories": ["kept", "dropped"],
            "values": [3, -1],
        }));
        assert!(svg.contains("<circle"), "{svg}");
    }

    /// Lexical containment misses the case that matters: a symlink inside the
    /// workspace pointing out of it passes a prefix check, and both
    /// `create_dir_all` and the write then follow it.
    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_workspace_does_not_smuggle_the_chart_out() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        std::os::unix::fs::symlink(outside.path(), workspace.path().join("escape"))
            .expect("symlink");

        let err = exec_chart(
            &json!({"values": [1], "path": "escape/out.svg"}),
            workspace.path(),
        )
        .expect_err("a symlink out of the workspace must be refused");
        assert!(format!("{err}").contains("escapes the workspace"), "{err}");
        assert!(
            !outside.path().join("out.svg").exists(),
            "the chart was written outside the workspace anyway"
        );
    }

    #[test]
    fn a_chart_cannot_be_written_outside_the_workspace() {
        let dir = tempfile::tempdir().expect("temp dir");
        let err = exec_chart(
            &json!({"values": [1], "path": "../escaped.svg"}),
            dir.path(),
        )
        .expect_err("must be refused");
        assert!(format!("{err}").contains("`..`"), "{err}");
    }

    /// `..` behind a directory that does not exist yet.
    ///
    /// The parent cannot be canonicalised, so a resolve-then-compare falls
    /// back to the literal path, which passes a prefix check — and then
    /// `create_dir_all` makes the missing directory and the write resolves the
    /// `..` for real, outside the workspace.
    #[test]
    fn a_dotdot_behind_a_missing_directory_is_still_refused() {
        let workspace = tempfile::tempdir().expect("workspace");
        let err = exec_chart(
            &json!({"values": [1], "path": "new/../../escaped.svg"}),
            workspace.path(),
        )
        .expect_err("must be refused");
        assert!(format!("{err}").contains("`..`"), "{err}");

        let outside = workspace.path().parent().expect("a parent");
        assert!(
            !outside.join("escaped.svg").exists(),
            "the chart was written outside the workspace anyway"
        );
    }

    /// A symlink that is not the immediate parent.
    ///
    /// The first version checked only the parent directory, so `link/out.svg`
    /// was caught but `link/sub/out.svg` was not: `sub` does not exist, the
    /// parent will not resolve, and the symlink one level further up never got
    /// looked at.
    #[cfg(unix)]
    #[test]
    fn a_symlink_above_a_missing_directory_is_refused() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        std::os::unix::fs::symlink(outside.path(), workspace.path().join("link")).expect("symlink");

        let err = exec_chart(
            &json!({"values": [1], "path": "link/sub/out.svg"}),
            workspace.path(),
        )
        .expect_err("a symlink at any depth must be refused");
        assert!(format!("{err}").contains("escapes the workspace"), "{err}");
        assert!(
            !outside.path().join("sub").exists(),
            "create_dir_all followed the symlink out"
        );
    }

    /// A pie draws one set of proportions. Rendering the first series and
    /// discarding the rest lost data silently, while the success message still
    /// claimed every series had been drawn.
    #[test]
    fn a_pie_of_several_series_is_refused_rather_than_truncated() {
        let dir = tempfile::tempdir().expect("temp dir");
        let err = exec_chart(
            &json!({
                "type": "pie",
                "series": [
                    {"name": "a", "values": [1, 2]},
                    {"name": "b", "values": [3, 4]},
                ]
            }),
            dir.path(),
        )
        .expect_err("must be refused");
        let msg = format!("{err}");
        assert!(msg.contains("one set of proportions"), "{msg}");
        assert!(
            msg.contains("bar"),
            "the message should offer the alternative: {msg}"
        );
    }

    /// An empty series among full ones used to pass, because the check asked
    /// whether *every* series was empty. For a pie the empty one is the one
    /// that gets drawn.
    #[test]
    fn one_empty_series_among_others_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let err = exec_chart(
            &json!({
                "series": [
                    {"name": "a", "values": []},
                    {"name": "b", "values": [1, 2]},
                ]
            }),
            dir.path(),
        )
        .expect_err("must be refused");
        assert!(format!("{err}").contains("no values"), "{err}");
    }

    #[test]
    fn an_unknown_chart_type_names_the_valid_ones() {
        let dir = tempfile::tempdir().expect("temp dir");
        let err = exec_chart(&json!({"type": "sunburst", "values": [1]}), dir.path())
            .expect_err("must be refused");
        let msg = format!("{err}");
        assert!(msg.contains("bar") && msg.contains("pie"), "{msg}");
    }

    /// Multi-series charts get a legend; a single series does not need one.
    #[test]
    fn a_legend_appears_only_when_there_is_something_to_distinguish() {
        let one = render(json!({"values": [1, 2], "name": "solo"}));
        assert!(!one.contains(">solo<"), "{one}");

        let two = render(json!({
            "series": [
                {"name": "alpha", "values": [1, 2]},
                {"name": "beta", "values": [2, 1]},
            ]
        }));
        assert!(two.contains(">alpha<") && two.contains(">beta<"), "{two}");
    }
}
