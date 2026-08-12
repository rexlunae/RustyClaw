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
use crate::ignore::Ignore;

/// Append a formatted fragment to the SVG buffer.
///
/// `write!` into a `String` returns a `Result` that cannot be an error — the
/// only failure `fmt::Write` has is one the underlying writer raises, and a
/// `String` raises none. The discard is written down here, once, rather than
/// spelled out at each of the twenty-odd call sites. The `_ = write!(…)` this
/// replaces slipped past `clippy::let_underscore_must_use` while still being
/// an undocumented discard, which is what STYLE_GUIDE.md is about.
macro_rules! svg {
    ($buf:expr, $($arg:tt)*) => {
        write!($buf, $($arg)*).ignore()
    };
}

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

/// Legend metrics. `CHAR_W` approximates the advance width of the 11px font
/// and is deliberately generous: overestimating costs a little whitespace,
/// underestimating puts an entry back over the edge.
const ROW_HEIGHT: f64 = 15.0;
const SWATCH: f64 = 10.0;
const GAP_AFTER_SWATCH: f64 = 5.0;
const GAP_BETWEEN: f64 = 18.0;
const CHAR_W: f64 = 6.6;

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

    // The same guard `write_file` applies, for the same reason. Workspace
    // containment is not enough on its own: an installation whose workspace
    // sits at or above the settings directory would let a model-supplied path
    // reach the vault, and "it is only a chart" is not a category the
    // filesystem recognises.
    if super::helpers::is_protected_path(&out) {
        return Err(ToolError::msg(super::helpers::VAULT_ACCESS_DENIED));
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ToolError::context("Could not create the chart's directory", e))?;
    }

    // Written through a verified descriptor rather than by path. Between
    // resolving the path above and opening it, a directory component can be
    // swapped for a symlink; `open_file_write_safe` re-resolves and hands back
    // the file it actually opened, so the write lands where the check looked.
    let (mut file, opened) = super::helpers::open_file_write_safe(&out)
        .map_err(|e| ToolError::context("Could not open the chart for writing", e))?;
    let root = workspace_dir
        .canonicalize()
        .unwrap_or_else(|_| workspace_dir.to_path_buf());
    if !opened.starts_with(&root) {
        return Err(ToolError::msg(format!(
            "chart path {} escapes the workspace",
            opened.display()
        )));
    }
    std::io::Write::write_all(&mut file, svg.as_bytes())
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
    // `symlink_metadata`, not `exists`. `exists` follows the link and answers
    // false for a dangling one, so a broken symlink as the final component
    // looked absent, the walk stopped at its parent — legitimately inside the
    // workspace — and the check passed. On Linux the later `O_NOFOLLOW` open
    // caught it; on macOS there is no such open, and the write created the
    // link's target outside the workspace. lstat sees the link itself, so the
    // walk stops there and `canonicalize` below refuses it.
    let mut existing = candidate.as_path();
    while std::fs::symlink_metadata(existing).is_err() {
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
    // Nothing left to append when the target itself already exists — the
    // second render to the same path. `join("")` still adds a separator, so
    // the reported location came back as `…/chart.svg/`, which the write
    // survived but every later `open` of that literal path does not.
    if rest.as_os_str().is_empty() {
        return Ok(anchor);
    }
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
            // A quoted number is still a number. Providers differ in how
            // strictly they type function arguments, and `"3"` is unambiguous
            // — parsing it is not the silent coercion this function refuses
            // elsewhere, because a string that is *not* a number still errors
            // rather than being dropped or read as zero.
            item.as_f64()
                .or_else(|| item.as_str().and_then(|t| t.trim().parse::<f64>().ok()))
                // NaN and the infinities parse happily and are not data.
                // `f64::min`/`max` ignore NaN outright, one `inf` collapses the
                // whole range to 0..1 so every honest bar is drawn off-plot,
                // and the coordinate maths then writes the literal text `NaN`
                // into an SVG attribute, which renderers drop — a blank or
                // silently mis-scaled picture, never an error. `data_label`
                // already guarded its half of this; the geometry did not.
                .filter(|v| v.is_finite())
                .ok_or_else(|| {
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

/// Format a number a person will read: tick labels, tooltips, percentages.
///
/// Separate from [`n`], which exists for SVG coordinates. Two decimals is
/// plenty for a coordinate and wrong for data: a chart of rates or ratios
/// draws the correct shape while every label reads "0", which is worse than
/// an error because the picture looks fine. This keeps roughly four
/// significant figures, so the magnitude survives whatever it is.
fn data_label(v: f64) -> String {
    if !v.is_finite() {
        return "—".to_string();
    }
    if v == 0.0 {
        return "0".to_string();
    }
    let magnitude = v.abs();
    // Beyond these bounds the fixed form is either unreadably long or a wall
    // of zeroes, and exponent notation is the honest rendering.
    if !(1e-4..1e7).contains(&magnitude) {
        return format!("{v:.3e}");
    }
    let decimals = (3 - magnitude.log10().floor() as i32).clamp(0, 12) as usize;
    let text = format!("{v:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
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
    //
    // Widened relative to the magnitude, not by a literal 1. Past 2^53 the
    // gap between representable doubles exceeds 1, so `lo - 1.0 == lo` and
    // the range comes back just as degenerate as it went in — then
    // `(v - lo) / (hi - lo)` is 0/0, `n()` writes the text `NaN` into a
    // coordinate, and the renderer drops the element. A flat series of large
    // numbers came out blank with no error.
    if (hi - lo).abs() < f64::EPSILON {
        let pad = lo.abs().max(1.0) * 0.5;
        return (lo - pad, hi + pad);
    }
    (lo, hi)
}

/// Document header, background and title.
fn open_svg(title: &str, canvas_h: f64) -> String {
    let mut s = String::new();
    svg!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{}" viewBox="0 0 {WIDTH} {}" font-family="system-ui, -apple-system, Segoe UI, sans-serif">"#,
        n(canvas_h),
        n(canvas_h)
    );
    // An explicit ground: without it the chart borrows the viewer's
    // background, and dark-theme clients get near-black text on near-black.
    svg!(
        s,
        r#"<rect width="{WIDTH}" height="{}" fill="{GROUND}"/>"#,
        n(canvas_h)
    );
    if !title.is_empty() {
        svg!(
            s,
            r#"<text x="{}" y="28" text-anchor="middle" font-size="17" font-weight="600" fill="{INK}">{}</text>"#,
            WIDTH / 2.0,
            esc(title)
        );
    }
    s
}

/// Axes, gridlines, tick labels and axis titles.
fn axes(svg: &mut String, lo: f64, hi: f64, x_label: &str, y_label: &str, canvas_h: f64) {
    let plot_h = HEIGHT - MARGIN_TOP - MARGIN_BOTTOM;
    let plot_w = WIDTH - MARGIN_LEFT - MARGIN_RIGHT;
    const TICKS: usize = 5;

    for i in 0..=TICKS {
        let frac = i as f64 / TICKS as f64;
        let y = MARGIN_TOP + plot_h * (1.0 - frac);
        let value = lo + (hi - lo) * frac;
        svg!(
            svg,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{GRID}" stroke-width="1"/>"#,
            n(MARGIN_LEFT),
            n(y),
            n(MARGIN_LEFT + plot_w),
            n(y)
        );
        svg!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="end" font-size="11" fill="{MUTED}">{}</text>"#,
            n(MARGIN_LEFT - 8.0),
            n(y + 4.0),
            data_label(value)
        );
    }
    svg!(
        svg,
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{INK}" stroke-width="1.5"/>"#,
        n(MARGIN_LEFT),
        n(MARGIN_TOP),
        n(MARGIN_LEFT),
        n(MARGIN_TOP + plot_h)
    );
    svg!(
        svg,
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{INK}" stroke-width="1.5"/>"#,
        n(MARGIN_LEFT),
        n(MARGIN_TOP + plot_h),
        n(MARGIN_LEFT + plot_w),
        n(MARGIN_TOP + plot_h)
    );
    if !x_label.is_empty() {
        svg!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="middle" font-size="12" fill="{MUTED}">{}</text>"#,
            n(MARGIN_LEFT + plot_w / 2.0),
            n(canvas_h - 14.0),
            esc(x_label)
        );
    }
    if !y_label.is_empty() {
        svg!(
            svg,
            r#"<text transform="translate(16,{}) rotate(-90)" text-anchor="middle" font-size="12" fill="{MUTED}">{}</text>"#,
            n(MARGIN_TOP + plot_h / 2.0),
            esc(y_label)
        );
    }
}

/// Series legend, drawn along the bottom.
fn legend_layout(series: &[Series]) -> Vec<Vec<(usize, String)>> {
    if series.len() < 2 {
        return Vec::new();
    }
    let right_edge = WIDTH - MARGIN_RIGHT;
    let mut rows: Vec<Vec<(usize, String)>> = vec![Vec::new()];
    let mut x = MARGIN_LEFT;
    for (i, s) in series.iter().enumerate() {
        let text = elide(&s.label);
        let width = entry_width(&text);
        // `x > MARGIN_LEFT` so a single entry too wide for the whole canvas
        // still gets its own row rather than looping forever on an empty one.
        if x + width > right_edge && x > MARGIN_LEFT {
            rows.push(Vec::new());
            x = MARGIN_LEFT;
        }
        rows.last_mut().expect("a row exists").push((i, text));
        x += width;
    }
    rows
}

/// How much taller the canvas has to be to hold a wrapped legend.
///
/// The first row fits in the existing bottom margin; each additional one does
/// not. Growing the canvas rather than moving the legend up is the whole
/// point: the first version raised the baseline by a row per extra row, which
/// put row zero at y=391 — directly on the category labels at y=392 — so the
/// key and the axis labels printed over each other. Space that does not exist
/// cannot be reclaimed by moving into it.
fn legend_extra_height(rows: &[Vec<(usize, String)>]) -> f64 {
    ROW_HEIGHT * rows.len().saturating_sub(1) as f64
}

/// Draw a laid-out legend, rows running downward from the usual baseline.
fn draw_legend(svg: &mut String, rows: &[Vec<(usize, String)>]) {
    for (r, row) in rows.iter().enumerate() {
        let y = HEIGHT - 34.0 + r as f64 * ROW_HEIGHT;
        let mut x = MARGIN_LEFT;
        for (i, text) in row {
            let colour = PALETTE[i % PALETTE.len()];
            svg!(
                svg,
                r#"<rect x="{}" y="{}" width="{SWATCH}" height="{SWATCH}" fill="{colour}"/>"#,
                n(x),
                n(y - 9.0)
            );
            svg!(
                svg,
                r#"<text x="{}" y="{}" font-size="11" fill="{INK}">{}</text>"#,
                n(x + SWATCH + GAP_AFTER_SWATCH),
                n(y),
                esc(text)
            );
            x += entry_width(text);
        }
    }
}

/// Horizontal space one legend entry occupies, swatch and trailing gap
/// included. Shared by the layout and the drawing so the two cannot disagree
/// about where the next entry starts.
fn entry_width(text: &str) -> f64 {
    SWATCH + GAP_AFTER_SWATCH + CHAR_W * text.chars().count() as f64 + GAP_BETWEEN
}

/// Shorten a series name that would crowd out everything beside it.
///
/// Counted in `char`s, not bytes: slicing a multi-byte name by byte offset
/// panics, and a label is exactly the sort of string that arrives with an
/// accent or an emoji in it.
fn elide(label: &str) -> String {
    const MAX: usize = 22;
    if label.chars().count() <= MAX {
        return label.to_string();
    }
    let kept: String = label.chars().take(MAX - 1).collect();
    format!("{}…", kept.trim_end())
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
    // Laid out before anything is drawn: the canvas has to be tall enough
    // for however many rows the legend needs, and the axis title sits at the
    // real bottom rather than a fixed one.
    let rows = legend_layout(series);
    let canvas_h = HEIGHT + legend_extra_height(&rows);
    let mut svg = open_svg(title, canvas_h);
    let (lo, hi) = value_range(series, true);
    axes(&mut svg, lo, hi, x_label, y_label, canvas_h);

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
            svg!(
                svg,
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{colour}"><title>{}: {}</title></rect>"#,
                n(x),
                n(top),
                n(bar_w),
                n(height.max(0.0)),
                esc(&s.label),
                data_label(v)
            );
        }
    }
    for i in 0..slots {
        svg!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="middle" font-size="11" fill="{MUTED}">{}</text>"#,
            n(MARGIN_LEFT + slot_w * (i as f64 + 0.5)),
            n(MARGIN_TOP + plot_h + 16.0),
            esc(&category(categories, i))
        );
    }
    draw_legend(&mut svg, &rows);
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
    // Laid out before anything is drawn: the canvas has to be tall enough
    // for however many rows the legend needs, and the axis title sits at the
    // real bottom rather than a fixed one.
    let rows = legend_layout(series);
    let canvas_h = HEIGHT + legend_extra_height(&rows);
    let mut svg = open_svg(title, canvas_h);
    let (lo, hi) = value_range(series, false);
    axes(&mut svg, lo, hi, x_label, y_label, canvas_h);

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
            svg!(
                svg,
                r#"<polyline points="{}" fill="none" stroke="{colour}" stroke-width="2" stroke-linejoin="round"/>"#,
                path.join(" ")
            );
        }
        for ((x, y), v) in pts.iter().zip(&s.values) {
            svg!(
                svg,
                r#"<circle cx="{}" cy="{}" r="3.5" fill="{colour}"><title>{}: {}</title></circle>"#,
                n(*x),
                n(*y),
                esc(&s.label),
                data_label(*v)
            );
        }
    }
    for i in 0..slots {
        svg!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="middle" font-size="11" fill="{MUTED}">{}</text>"#,
            n(x_at(i)),
            n(MARGIN_TOP + plot_h + 16.0),
            esc(&category(categories, i))
        );
    }
    draw_legend(&mut svg, &rows);
    svg.push_str("</svg>");
    svg
}

fn render_pie(title: &str, categories: &[String], series: &Series) -> String {
    let mut svg = open_svg(title, HEIGHT);
    // Negative slices have no meaning in a pie — a share of a whole cannot be
    // below nothing — so they are dropped rather than drawn inside out.
    let values: Vec<f64> = series.values.iter().map(|v| v.max(0.0)).collect();
    let total: f64 = values.iter().sum();
    let cx = WIDTH / 2.0 - 80.0;
    let cy = MARGIN_TOP + (HEIGHT - MARGIN_TOP - MARGIN_BOTTOM) / 2.0;
    let r = 140.0_f64;

    if total <= 0.0 {
        svg!(
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
            svg!(
                svg,
                r#"<circle cx="{}" cy="{}" r="{r}" fill="{colour}"><title>{}: {} (100%)</title></circle>"#,
                n(cx),
                n(cy),
                esc(&label),
                data_label(v)
            );
            angle = end;
            continue;
        }
        svg!(
            svg,
            r#"<path d="M {} {} L {} {} A {r} {r} 0 {large} 1 {} {} Z" fill="{colour}"><title>{}: {} ({}%)</title></path>"#,
            n(cx),
            n(cy),
            n(x0),
            n(y0),
            n(x1),
            n(y1),
            esc(&label),
            data_label(v),
            data_label(v / total * 100.0)
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
        svg!(
            svg,
            r#"<rect x="{}" y="{}" width="11" height="11" fill="{colour}"/>"#,
            n(WIDTH - 190.0),
            n(ly - 9.0)
        );
        svg!(
            svg,
            r#"<text x="{}" y="{}" font-size="12" fill="{INK}">{} ({}%)</text>"#,
            n(WIDTH - 173.0),
            n(ly),
            esc(&category(categories, i)),
            data_label(v / total * 100.0)
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

    /// Two decimals is right for an SVG coordinate and wrong for data. A chart
    /// of rates or ratios drew the correct shape while every tick label and
    /// tooltip read "0" — worse than an error, because the picture looks fine.
    #[test]
    fn small_numbers_keep_their_magnitude_in_labels() {
        let svg = render(json!({
            "type": "line",
            "categories": ["a", "b", "c"],
            "values": [0.001, 0.002, 0.003],
        }));
        assert!(
            svg.contains("0.001"),
            "small values collapsed to zero: {svg}"
        );
        assert!(svg.contains("0.003"), "{svg}");
    }

    /// Round numbers must not gain a tail of zeroes for it.
    #[test]
    fn ordinary_numbers_stay_short() {
        assert_eq!(data_label(5.0), "5");
        assert_eq!(data_label(0.0), "0");
        assert_eq!(data_label(1234.0), "1234");
        assert_eq!(data_label(33.333_333), "33.33");
        assert_eq!(data_label(0.5), "0.5");
    }

    /// Past the point where a fixed form is a wall of zeroes, exponent
    /// notation is the honest rendering.
    #[test]
    fn extreme_magnitudes_fall_back_to_exponent_form() {
        assert!(
            data_label(0.000_000_12).contains('e'),
            "{}",
            data_label(0.000_000_12)
        );
        assert!(data_label(9.9e12).contains('e'), "{}", data_label(9.9e12));
        // And a non-finite value renders as a dash rather than "NaN" on a chart.
        assert_eq!(data_label(f64::NAN), "—");
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

    /// The schema handed to the model must describe what the tool accepts. It
    /// advertised `values` as a list of strings while the executor demanded
    /// numbers, so a provider enforcing its own schema would send exactly the
    /// shape the tool then rejected.
    #[test]
    fn the_advertised_item_types_match_what_the_tool_accepts() {
        let params = crate::tools::params::chart_params();
        let kind = |n: &str| {
            params
                .iter()
                .find(|p| p.name == n)
                .unwrap_or_else(|| panic!("no {n} parameter"))
                .param_type
                .clone()
        };
        assert_eq!(kind("values"), "array<number>");
        assert_eq!(kind("series"), "array<object>");
        // Categories really are text, so they stay a plain string array.
        assert_eq!(kind("categories"), "array");
    }

    /// A quoted number is still a number — providers differ on how strictly
    /// they type arguments. A string that is not a number still errors.
    #[test]
    fn quoted_numbers_are_accepted_but_words_are_not() {
        let svg = render(json!({"values": ["3", "5", 4]}));
        assert!(svg.contains("<rect"), "{svg}");

        let dir = tempfile::tempdir().expect("temp dir");
        let err = exec_chart(&json!({"values": ["three"]}), dir.path())
            .expect_err("a word is still not a number");
        assert!(format!("{err}").contains("not a number"), "{err}");
    }

    /// NaN and the infinities are not data. They parse, they survive
    /// `f64::min`/`max` (which ignore NaN), one `inf` collapses the range so
    /// every honest bar is drawn off-plot, and the coordinate maths writes the
    /// literal text `NaN` into an SVG attribute — a blank or mis-scaled
    /// picture with no error anywhere.
    #[test]
    fn non_finite_values_are_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        for bad in [
            json!({"values": [1, "NaN", 3]}),
            json!({"values": [1, "inf", 3]}),
            json!({"values": [1, "infinity", 3]}),
            json!({"values": [1, "1e999", 3]}),
        ] {
            let err = exec_chart(&bad, dir.path()).expect_err(&format!("{bad} should be refused"));
            assert!(format!("{err}").contains("not a number"), "{err}");
        }

        // And a finite chart never emits those literals into a coordinate.
        let svg = render(json!({"values": [1, 2, 3]}));
        assert!(!svg.contains("NaN") && !svg.contains("inf"), "{svg}");
    }

    /// Every legend entry has to be inside the viewport. Content past the
    /// right edge of a fixed-size SVG is not drawn at all, so a run-on row
    /// left later series with a colour in the plot and no key — and nothing
    /// indicating a key was missing.
    #[test]
    fn every_legend_entry_stays_inside_the_canvas() {
        let series: Vec<_> = (0..8)
            .map(|i| json!({"name": format!("series number {i}"), "values": [1, 2]}))
            .collect();
        let svg = render(json!({"series": series}));

        // Pull every legend swatch's x and confirm none is off-canvas.
        let mut checked = 0;
        for frag in svg.split("<rect x=\"").skip(1) {
            let x: f64 = frag
                .split('"')
                .next()
                .expect("an x value")
                .parse()
                .expect("a number");
            assert!(
                x < WIDTH - MARGIN_RIGHT,
                "a swatch sits at x={x}, off-canvas"
            );
            checked += 1;
        }
        assert!(checked >= 8, "expected 8 legend swatches, saw {checked}");

        // And all eight names are present, so none was dropped to make room.
        for i in 0..8 {
            assert!(
                svg.contains(&format!("series number {i}")),
                "series {i} missing"
            );
        }
    }

    /// The x-position test above passed while the rows were colliding
    /// vertically: with two rows the first landed at y=391, one pixel above
    /// the category labels at y=392, and its swatch covered them. Checking
    /// only one axis is how a wrapped legend looked correct.
    #[test]
    fn a_wrapped_legend_does_not_land_on_the_category_labels() {
        let series: Vec<_> = (0..8)
            .map(|i| json!({"name": format!("series number {i}"), "values": [1, 2]}))
            .collect();
        let svg = render(json!({"series": series, "categories": ["a", "b"]}));

        // The canvas has to have grown to hold the extra rows.
        let declared: f64 = svg
            .split("height=\"")
            .nth(1)
            .and_then(|f| f.split('"').next())
            .expect("a canvas height")
            .parse()
            .expect("a number");
        assert!(
            declared > HEIGHT,
            "canvas did not grow for the wrapped legend: {declared}"
        );

        // Category labels sit at MARGIN_TOP + plot_h + 16; no legend swatch
        // may overlap that band.
        let labels_y = MARGIN_TOP + (HEIGHT - MARGIN_TOP - MARGIN_BOTTOM) + 16.0;
        for frag in svg.split("<rect x=\"").skip(1) {
            let mut parts = frag.split('"');
            let _x: f64 = parts.next().expect("x").parse().expect("number");
            let y: f64 = parts.nth(1).expect("y").parse().expect("number");
            assert!(
                y + SWATCH <= labels_y || y > labels_y + 4.0,
                "a legend swatch at y={y} overlaps the category labels at {labels_y}"
            );
        }

        // And every entry is still inside the (now taller) canvas.
        assert!(
            svg.contains("series number 7"),
            "last series missing: {svg}"
        );
    }

    /// Past 2^53 the gap between doubles exceeds 1, so widening a flat range
    /// by a literal ±1 changes nothing and the coordinate maths divides by
    /// zero. Bars were spared because zero is folded in; lines were not.
    #[test]
    fn a_flat_series_of_huge_numbers_still_renders() {
        for chart in ["line", "scatter", "bar"] {
            let svg = render(json!({"type": chart, "values": [1e18, 1e18]}));
            assert!(!svg.contains("NaN"), "{chart} produced NaN: {svg}");
            assert!(!svg.contains("inf"), "{chart} produced inf: {svg}");
        }
        let single = render(json!({"type": "line", "values": [1e18]}));
        assert!(!single.contains("NaN"), "{single}");
    }

    /// A name long enough to fill the canvas on its own is shortened rather
    /// than pushing everything after it off the edge.
    #[test]
    fn an_overlong_series_name_is_elided() {
        let long = "an extraordinarily long series name that would never fit";
        let svg = render(json!({
            "series": [
                {"name": long, "values": [1, 2]},
                {"name": "short", "values": [2, 1]},
            ]
        }));
        // The legend entry is shortened…
        assert!(
            svg.contains(&format!("fill=\"{INK}\">an extraordinarily lo…</text>")),
            "the legend entry was not elided: {svg}"
        );
        // …while the tooltip keeps the whole name, which is the point of
        // eliding only the label.
        assert!(
            svg.contains(&format!("<title>{long}: 1</title>")),
            "the tooltip lost the full name: {svg}"
        );
        assert!(svg.contains(">short<"), "the second entry vanished: {svg}");
    }

    /// Eliding counts characters, not bytes — slicing a multi-byte name by
    /// byte offset panics.
    #[test]
    fn eliding_a_multibyte_name_does_not_panic() {
        let svg = render(json!({
            "series": [
                {"name": "日本語のとても長いシリーズ名前ですこれは切られます", "values": [1]},
                {"name": "b", "values": [2]},
            ]
        }));
        assert!(svg.contains('…'), "{svg}");
    }

    /// Re-rendering to the same path used to report `…/chart.svg/`. The write
    /// survived it; every later `open` of that literal path would not.
    #[test]
    fn re_rendering_reports_a_path_that_can_actually_be_opened() {
        let dir = tempfile::tempdir().expect("temp dir");
        exec_chart(&json!({"values": [1, 2]}), dir.path()).expect("first render");
        let second = exec_chart(&json!({"values": [3, 4]}), dir.path()).expect("second render");

        assert!(
            !second.contains(".svg/"),
            "reported a directory-shaped path: {second}"
        );
        // And the reported file is genuinely openable.
        let reported = second.rsplit(" to ").next().expect("a path in the message");
        assert!(
            std::fs::read_to_string(reported).is_ok(),
            "cannot open the path the tool reported: {reported}"
        );
    }

    /// A dangling symlink as the final component reads as absent to `exists`,
    /// so the walk stopped at its parent and the check passed. Linux caught it
    /// at `O_NOFOLLOW`; macOS has no such open and wrote the link's target.
    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_target_is_refused() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let target = outside.path().join("not-there-yet.svg");
        std::os::unix::fs::symlink(&target, workspace.path().join("dangling.svg"))
            .expect("symlink");

        let err = exec_chart(
            &json!({"values": [1], "path": "dangling.svg"}),
            workspace.path(),
        )
        .expect_err("a dangling symlink out of the workspace must be refused");
        assert!(!target.exists(), "the write followed the link: {err}");
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
