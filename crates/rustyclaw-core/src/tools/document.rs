//! Text extraction from Office documents (DOCX, XLSX, PPTX).
//!
//! `read_file` could only read these through macOS `textutil`, so on Linux
//! and Windows a business document was simply unreadable. This covers the
//! three OOXML formats on every platform.
//!
//! All three are ZIP archives of XML, so DOCX and PPTX are handled directly
//! with `zip` (already a dependency) plus a pull parser: both are a matter of
//! concatenating text runs in document order. XLSX gets `calamine`, because a
//! spreadsheet is not a text run — cells reach their values through a shared
//! string table, inline strings, or a formula result, and re-deriving that is
//! how subtly wrong output happens.
//!
//! PDF is deliberately not here; it has its own tool with page ranges and
//! metadata that these formats do not share.

use crate::tools::error::{ToolError, ToolResult};
use serde_json::{Value, json};
use std::io::Read;
use std::path::Path;
use tracing::{debug, instrument};

use super::helpers::{VAULT_ACCESS_DENIED, is_protected_path, resolve_path};

/// Maximum characters returned from one extraction.
///
/// Matches the `pdf` tool: a long document should truncate visibly rather
/// than flood the model's context.
const MAX_OUTPUT_CHARS: usize = 100_000;

/// A format this tool can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocKind {
    Docx,
    Xlsx,
    Pptx,
}

impl DocKind {
    /// Identify by extension. Content sniffing would not help: all three are
    /// ZIP archives, so the magic bytes are identical.
    fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "docx" | "docm" => Some(Self::Docx),
            "xlsx" | "xlsm" => Some(Self::Xlsx),
            "pptx" | "pptm" => Some(Self::Pptx),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
        }
    }
}

/// Whether `path` names a format this tool handles.
///
/// Used by `read_file` to route rich documents here instead of returning
/// bytes or failing.
pub fn is_office_document(path: &Path) -> bool {
    DocKind::from_path(path).is_some()
}

/// Execute the `document` tool.
#[instrument(skip(args, workspace_dir))]
pub fn exec_document(args: &Value, workspace_dir: &Path) -> ToolResult {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("extract");

    match action {
        "extract" => exec_extract(args, workspace_dir),
        "info" => exec_info(args, workspace_dir),
        other => Err(format!("Unknown document action '{other}'. Available: extract, info").into()),
    }
}

/// Resolve, guard, and classify the target path.
fn open_target(
    args: &Value,
    workspace_dir: &Path,
) -> Result<(std::path::PathBuf, DocKind), String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let path = resolve_path(workspace_dir, path_str);
    if is_protected_path(&path) {
        return Err(VAULT_ACCESS_DENIED.to_string());
    }
    if !path.exists() {
        return Err(format!("File not found: {}", path.display()));
    }

    let kind = DocKind::from_path(&path).ok_or_else(|| {
        format!(
            "'{}' is not a supported Office document. Handled: .docx, .xlsx, .pptx \
             (and their macro-enabled variants). Use the pdf tool for PDFs and \
             read_file for plain text.",
            path.display()
        )
    })?;
    Ok((path, kind))
}

fn exec_extract(args: &Value, workspace_dir: &Path) -> ToolResult {
    let (path, kind) = open_target(args, workspace_dir).map_err(ToolError::msg)?;
    debug!(path = %path.display(), kind = kind.label(), "Extracting document text");

    let text = match kind {
        DocKind::Docx => extract_docx(&path),
        DocKind::Xlsx => extract_xlsx(&path),
        DocKind::Pptx => extract_pptx(&path),
    }
    .map_err(ToolError::msg)?;

    if text.trim().is_empty() {
        return Ok(format!(
            "({} contains no extractable text)",
            kind.label().to_uppercase()
        ));
    }

    Ok(truncate(text))
}

fn exec_info(args: &Value, workspace_dir: &Path) -> ToolResult {
    let (path, kind) = open_target(args, workspace_dir).map_err(ToolError::msg)?;

    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let parts = archive_entries(&path).map_err(ToolError::msg)?;

    // Report the structural count that means something for the format: pages
    // are not a concept in any of these until they are rendered.
    let (unit, count) = match kind {
        DocKind::Docx => ("paragraphs", extract_docx(&path).map(|t| t.lines().count())),
        DocKind::Xlsx => ("sheets", sheet_names(&path).map(|s| s.len())),
        DocKind::Pptx => (
            "slides",
            Ok(parts
                .iter()
                .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
                .count()),
        ),
    };
    let count = count.map_err(ToolError::msg)?;

    let mut info = json!({
        "path": path.display().to_string(),
        "format": kind.label(),
        "size_bytes": size,
        unit: count,
    });
    if kind == DocKind::Xlsx {
        if let Ok(names) = sheet_names(&path) {
            info["sheet_names"] = json!(names);
        }
    }

    serde_json::to_string_pretty(&info)
        .map_err(|e| ToolError::context("Failed to serialize document info", e))
}

/// Names of every entry in the archive.
fn archive_entries(path: &Path) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Failed to open file: {e}"))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("Not a readable Office document (bad archive): {e}"))?;
    Ok((0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .collect())
}

/// Read one archive member as a UTF-8 string.
fn read_part(path: &Path, part: &str) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Failed to open file: {e}"))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("Not a readable Office document (bad archive): {e}"))?;
    let mut entry = zip
        .by_name(part)
        .map_err(|_| format!("Document is missing its {part} part"))?;
    let mut buf = String::new();
    entry
        .read_to_string(&mut buf)
        .map_err(|e| format!("Failed to read {part}: {e}"))?;
    Ok(buf)
}

/// Concatenate the text of every `tag` element, inserting a break wherever
/// `break_tag` closes.
///
/// Shared by DOCX and PPTX: both mark text runs with a single element type
/// and paragraph boundaries with another, differing only in namespace.
fn text_runs(xml: &str, tag: &[u8], break_tag: &[u8]) -> Result<String, String> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_str(xml);
    // Catches a closing tag that does not match its opener. It does *not*
    // catch tags simply left open at EOF, which is what a truncated file
    // looks like — `depth` below covers that.
    reader.config_mut().check_end_names = true;
    let mut out = String::new();
    let mut in_run = false;
    let mut depth: i32 = 0;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                if e.local_name().as_ref() == tag {
                    in_run = true;
                }
            }
            Ok(Event::End(e)) => {
                depth -= 1;
                let name = e.local_name();
                if name.as_ref() == tag {
                    in_run = false;
                } else if name.as_ref() == break_tag {
                    out.push('\n');
                }
            }
            Ok(Event::Text(e)) if in_run => {
                // Decode bytes, then turn &amp; and friends back into their
                // characters — without the second step the model reads raw
                // entities. In quick-xml 0.41 these are separate calls.
                // `xml_content` decodes *and* resolves entities, so &amp;
                // arrives as `&`. Unescaping again would treat that `&` as
                // the start of a fresh entity and swallow it.
                let text = e
                    .xml_content(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|err| format!("Undecodable text in document: {err}"))?;
                out.push_str(&text);
            }
            // quick-xml 0.41 reports entity references as their own event
            // rather than inlining them, so `&amp;` arrives here and not in
            // the surrounding Text. Ignoring it silently deletes the
            // character — an ampersand in a document simply vanished.
            Ok(Event::GeneralRef(e)) if in_run => {
                let name = e
                    .decode()
                    .map_err(|err| format!("Undecodable entity in document: {err}"))?;
                match quick_xml::escape::resolve_xml_entity(&name) {
                    Some(resolved) => out.push_str(resolved),
                    // Numeric references (&#8217; and friends) are not named
                    // entities; resolve them from the code point.
                    None => {
                        if let Some(ch) = numeric_entity(&name) {
                            out.push(ch);
                        }
                    }
                }
            }
            Ok(Event::Eof) => {
                // Tags still open means the file ends mid-element. Returning
                // the partial text would read as a short document rather than
                // a damaged one.
                if depth > 0 {
                    return Err(
                        "Document XML ends mid-element — the file looks truncated or corrupt"
                            .to_string(),
                    );
                }
                break;
            }
            Err(e) => return Err(format!("Malformed document XML: {e}")),
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn extract_docx(path: &Path) -> Result<String, String> {
    let xml = read_part(path, "word/document.xml")?;
    // w:t is a text run, w:p a paragraph.
    text_runs(&xml, b"t", b"p")
}

fn extract_pptx(path: &Path) -> Result<String, String> {
    let mut slides: Vec<String> = archive_entries(path)?
        .into_iter()
        .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
        .collect();
    // Zip order is arbitrary; slide2 before slide10 needs numeric ordering,
    // or a deck reads out of sequence.
    slides.sort_by_key(|n| slide_number(n).unwrap_or(usize::MAX));

    let mut out = String::new();
    for (idx, part) in slides.iter().enumerate() {
        let xml = read_part(path, part)?;
        // a:t is a text run, a:p a paragraph.
        let text = text_runs(&xml, b"t", b"p")?;
        if text.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("--- Slide {} ---\n", idx + 1));
        out.push_str(text.trim());
        out.push_str("\n\n");
    }
    Ok(out)
}

/// Resolve a numeric character reference body (`#8217` or `#x2019`).
///
/// Word and PowerPoint emit curly quotes and dashes this way constantly, so
/// dropping them would pepper extracted prose with holes.
fn numeric_entity(body: &str) -> Option<char> {
    let digits = body.strip_prefix('#')?;
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse().ok()?,
    };
    char::from_u32(code)
}

/// The number in `ppt/slides/slideN.xml`.
fn slide_number(name: &str) -> Option<usize> {
    name.rsplit_once("slide")?
        .1
        .strip_suffix(".xml")?
        .parse()
        .ok()
}

fn sheet_names(path: &Path) -> Result<Vec<String>, String> {
    use calamine::Reader;
    let workbook = calamine::open_workbook_auto(path)
        .map_err(|e| format!("Failed to open spreadsheet: {e}"))?;
    Ok(workbook.sheet_names().to_vec())
}

fn extract_xlsx(path: &Path) -> Result<String, String> {
    use calamine::Reader;

    let mut workbook = calamine::open_workbook_auto(path)
        .map_err(|e| format!("Failed to open spreadsheet: {e}"))?;
    let names = workbook.sheet_names().to_vec();

    let mut out = String::new();
    for name in names {
        let Ok(range) = workbook.worksheet_range(&name) else {
            continue;
        };
        if range.is_empty() {
            continue;
        }
        out.push_str(&format!("--- Sheet: {name} ---\n"));
        for row in range.rows() {
            // Tab-separated: a spreadsheet's shape is part of its meaning,
            // and this keeps columns aligned without inventing a table
            // format the model then has to parse back.
            let cells: Vec<String> = row.iter().map(|c| c.to_string()).collect();
            out.push_str(&cells.join("\t"));
            out.push('\n');
        }
        out.push('\n');
    }
    Ok(out)
}

/// Cut to [`MAX_OUTPUT_CHARS`], saying so rather than trailing off.
fn truncate(text: String) -> String {
    if text.chars().count() <= MAX_OUTPUT_CHARS {
        return text;
    }
    let kept: String = text.chars().take(MAX_OUTPUT_CHARS).collect();
    format!("{kept}\n\n[… truncated at {MAX_OUTPUT_CHARS} characters …]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_the_formats_it_handles() {
        assert!(is_office_document(Path::new("/tmp/report.docx")));
        assert!(is_office_document(Path::new("/tmp/budget.XLSX")));
        assert!(is_office_document(Path::new("/tmp/deck.pptx")));
        // Macro-enabled variants are the same container.
        assert!(is_office_document(Path::new("/tmp/m.docm")));
        // PDF has its own tool; plain text needs no tool at all.
        assert!(!is_office_document(Path::new("/tmp/paper.pdf")));
        assert!(!is_office_document(Path::new("/tmp/notes.txt")));
        assert!(!is_office_document(Path::new("/tmp/noext")));
    }

    #[test]
    fn slides_order_numerically_not_lexically() {
        let mut names = vec![
            "ppt/slides/slide10.xml".to_string(),
            "ppt/slides/slide2.xml".to_string(),
            "ppt/slides/slide1.xml".to_string(),
        ];
        names.sort_by_key(|n| slide_number(n).unwrap_or(usize::MAX));
        assert_eq!(
            names,
            vec![
                "ppt/slides/slide1.xml",
                "ppt/slides/slide2.xml",
                "ppt/slides/slide10.xml"
            ],
            "lexical order would read slide10 before slide2"
        );
    }

    #[test]
    fn text_runs_join_runs_and_break_paragraphs() {
        // Word splits a sentence across runs whenever formatting changes, so
        // joining them is the whole job.
        let xml = r#"<w:document xmlns:w="x"><w:body>
            <w:p><w:r><w:t>Hello </w:t></w:r><w:r><w:t>world</w:t></w:r></w:p>
            <w:p><w:r><w:t>Second &amp; last</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let text = text_runs(xml, b"t", b"p").expect("well-formed");
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(lines, vec!["Hello world", "Second & last"]);
    }

    #[test]
    fn malformed_xml_is_an_error_not_a_panic() {
        let result = text_runs("<w:p><w:t>unclosed", b"t", b"p");
        assert!(
            result.is_err(),
            "a truncated document must report, not panic"
        );
    }

    #[test]
    fn truncation_announces_itself() {
        let long = "x".repeat(MAX_OUTPUT_CHARS + 10);
        let out = truncate(long);
        assert!(
            out.contains("truncated"),
            "silent truncation reads as a short document"
        );
    }

    #[test]
    fn a_missing_file_reports_the_path() {
        let args = json!({"path": "definitely-not-here.docx"});
        let err = exec_document(&args, Path::new("/tmp"))
            .expect_err("missing file must error")
            .to_string();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn an_unsupported_extension_points_at_the_right_tool() {
        let dir = std::env::temp_dir().join("rustyclaw-doc-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("paper.pdf");
        std::fs::write(&file, b"%PDF-1.4").expect("write");

        let args = json!({"path": file.display().to_string()});
        let err = exec_document(&args, &dir)
            .expect_err("pdf is not this tool's job")
            .to_string();
        assert!(err.contains("pdf tool"), "should redirect, got: {err}");

        std::fs::remove_file(&file).ok();
    }

    // ── Round-trip over real archives ───────────────────────────────────
    //
    // The unit tests above exercise the XML parsing on snippets; these build
    // genuine OOXML containers so the zip plumbing, part naming, and slide
    // ordering are covered too. Hand-built rather than fixture files so the
    // expected text is visible right next to the assertion.

    fn write_zip(path: &Path, parts: &[(&str, &str)]) {
        use std::io::Write;
        let file = std::fs::File::create(path).expect("create archive");
        let mut zip = zip::ZipWriter::new(file);
        let opts: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in parts {
            zip.start_file(*name, opts).expect("start entry");
            zip.write_all(body.as_bytes()).expect("write entry");
        }
        zip.finish().expect("finish archive");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("rustyclaw-doc-roundtrip");
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.join(name)
    }

    #[test]
    fn extracts_prose_from_a_real_docx() {
        let path = scratch("round.docx");
        write_zip(
            &path,
            &[(
                "word/document.xml",
                r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body>
                   <w:p><w:r><w:t>Quarterly </w:t></w:r><w:r><w:t>report</w:t></w:r></w:p>
                   <w:p><w:r><w:t>Revenue &amp; costs</w:t></w:r></w:p>
                   </w:body></w:document>"#,
            )],
        );

        let out = exec_document(&json!({"path": path.display().to_string()}), &scratch(""))
            .expect("extraction should succeed");
        assert!(out.contains("Quarterly report"), "runs must join: {out}");
        assert!(
            out.contains("Revenue & costs"),
            "entities must survive: {out}"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn reads_slides_in_deck_order() {
        let path = scratch("round.pptx");
        let slide = |text: &str| {
            format!(
                r#"<?xml version="1.0"?><p:sld xmlns:a="a"><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:sld>"#
            )
        };
        // Deliberately added out of order, and with a two-digit slide, since
        // zip order is arbitrary and lexical sorting puts 10 before 2.
        write_zip(
            &path,
            &[
                ("ppt/slides/slide10.xml", &slide("Tenth")),
                ("ppt/slides/slide2.xml", &slide("Second")),
                ("ppt/slides/slide1.xml", &slide("First")),
            ],
        );

        let out = exec_document(&json!({"path": path.display().to_string()}), &scratch(""))
            .expect("extraction should succeed");
        let first = out.find("First").expect("slide 1 present");
        let second = out.find("Second").expect("slide 2 present");
        let tenth = out.find("Tenth").expect("slide 10 present");
        assert!(first < second && second < tenth, "deck order wrong:\n{out}");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn info_counts_slides_without_extracting() {
        let path = scratch("info.pptx");
        let body =
            r#"<?xml version="1.0"?><p:sld xmlns:a="a"><a:p><a:r><a:t>x</a:t></a:r></a:p></p:sld>"#;
        write_zip(
            &path,
            &[
                ("ppt/slides/slide1.xml", body),
                ("ppt/slides/slide2.xml", body),
                // Not a slide: must not be counted.
                ("ppt/slideLayouts/slideLayout1.xml", body),
            ],
        );

        let out = exec_document(
            &json!({"path": path.display().to_string(), "action": "info"}),
            &scratch(""),
        )
        .expect("info should succeed");
        let parsed: Value = serde_json::from_str(&out).expect("info returns json");
        assert_eq!(parsed["slides"], 2, "layouts are not slides: {out}");
        assert_eq!(parsed["format"], "pptx");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_non_archive_with_the_right_extension_reports_clearly() {
        let path = scratch("fake.docx");
        std::fs::write(&path, b"this is not a zip").expect("write");

        let err = exec_document(&json!({"path": path.display().to_string()}), &scratch(""))
            .expect_err("a renamed text file is not a docx")
            .to_string();
        assert!(
            err.contains("archive"),
            "should name the real problem: {err}"
        );

        std::fs::remove_file(&path).ok();
    }
}
