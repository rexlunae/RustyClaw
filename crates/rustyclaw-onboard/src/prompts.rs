//! (onboard submodule)

use std::io::{self, BufRead, Write};

use anyhow::Result;
use crossterm::terminal;

use rustyclaw_core::theme as t;

/// Render a QR code as compact Unicode art in the terminal.
///
/// Uses Unicode half-block characters (▀▄█ and space) so that each
/// character cell encodes two vertical "modules", giving a clean,
/// scannable result at roughly half the height of a naive render.
pub(crate) fn print_qr_code(data: &str) {
    use qrcode::QrCode;

    let code = match QrCode::new(data) {
        Ok(c) => c,
        Err(_) => {
            // Silently fall back — the URL is printed below anyway.
            return;
        }
    };

    let colors = code.to_colors();
    let width = code.width();

    // Quiet zone: 4 modules on each side (QR spec recommends 4).
    let qz = 4;
    let total_w = width + 2 * qz;

    // Collect rows with quiet-zone padding (false = light module).
    let mut rows: Vec<Vec<bool>> = Vec::new();
    for _ in 0..qz {
        rows.push(vec![false; total_w]);
    }
    for y in 0..width {
        let mut row = vec![false; qz];
        for x in 0..width {
            row.push(colors[y * width + x] == qrcode::Color::Dark);
        }
        row.resize(total_w, false);
        rows.push(row);
    }
    for _ in 0..qz {
        rows.push(vec![false; total_w]);
    }

    // Render two rows at a time using Unicode half-block characters.
    //
    // We use an INVERTED scheme so it works on dark terminal backgrounds:
    //   - Light module (false) → print a block character (appears as
    //     the foreground colour = white)
    //   - Dark  module (true)  → print a space (shows the background
    //     colour = dark/black)
    //
    // This way the QR's dark modules are dark and the light modules
    // (including the quiet zone) are bright — good contrast for scanners.
    //
    // Half-block ▀ means "top pixel on, bottom pixel off" (in the
    // inverted world: top is light, bottom is dark).
    let total_h = rows.len();
    let indent = "  ";
    for pair in (0..total_h).step_by(2) {
        print!("{}", indent);
        for (x, &top_dark) in rows[pair].iter().enumerate() {
            let bot_dark = if pair + 1 < total_h {
                rows[pair + 1][x]
            } else {
                false
            };
            // Invert: light → filled, dark → empty
            let ch = match (top_dark, bot_dark) {
                (false, false) => '█', // both light → full block
                (false, true) => '▀',  // top light, bottom dark → upper half
                (true, false) => '▄',  // top dark, bottom light → lower half
                (true, true) => ' ',   // both dark → space
            };
            print!("{}", ch);
        }
        println!();
    }
}

pub(crate) fn prompt_line(reader: &mut impl BufRead, prompt: &str) -> Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;
    let mut buf = String::new();
    reader.read_line(&mut buf)?;
    Ok(buf
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_string())
}

/// Best-effort username for TOTP account labels.
pub(crate) fn whoami() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string())
}

/// Interactive fuzzy-search selector for large lists.
///
/// Shows a filter input at the top. As the user types, items are fuzzy-matched
/// and the list updates in real time. Arrow keys navigate, Enter selects,
/// Esc cancels.
///
/// Returns the *original* index of the selected item (into `items`), or `None`
/// if cancelled.
pub(crate) fn fuzzy_select(items: &[impl AsRef<str>], heading_text: &str) -> Result<Option<usize>> {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
    use crossterm::{cursor, execute, terminal as ct};
    use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
    use nucleo_matcher::{Config, Matcher};

    if items.is_empty() {
        return Ok(None);
    }

    let max_visible: usize = 14;

    // Print the heading (above the interactive region)
    println!("{}", t::heading(heading_text));
    println!();

    // We'll draw: 1 line for filter input + max_visible lines for items + 1 hint line
    let draw_height = 1 + max_visible + 1;

    // Pre-allocate the lines so we can overwrite them.
    for _ in 0..draw_height {
        println!();
    }

    let mut stdout = io::stdout();
    let mut matcher = Matcher::new(Config::DEFAULT);

    // Helper: compute filtered indices based on current query
    let compute_matches = |query: &str, matcher: &mut Matcher| -> Vec<(usize, u32)> {
        if query.is_empty() {
            // No filter — return all items with score 0
            return items.iter().enumerate().map(|(i, _)| (i, 0u32)).collect();
        }

        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut results: Vec<(usize, u32)> = Vec::new();
        let mut buf = Vec::new();

        for (i, item) in items.iter().enumerate() {
            buf.clear();
            let haystack = nucleo_matcher::Utf32Str::new(item.as_ref(), &mut buf);
            if let Some(score) = pattern.score(haystack, matcher) {
                results.push((i, score));
            }
        }

        // Sort by score descending (best matches first)
        results.sort_by_key(|r| std::cmp::Reverse(r.1));
        results
    };

    // Helper: draw the current state
    let draw = |stdout: &mut io::Stdout,
                query: &str,
                matches: &[(usize, u32)],
                selected: usize,
                scroll_offset: usize|
     -> io::Result<()> {
        // Move cursor up to the first line of our region
        execute!(stdout, cursor::MoveUp(draw_height as u16))?;

        // Filter input line
        execute!(stdout, ct::Clear(ct::ClearType::CurrentLine))?;
        write!(
            stdout,
            "  {} {}{}\r\n",
            t::accent("Filter:"),
            query,
            t::muted("▌") // cursor indicator
        )?;

        // Item lines
        let end = (scroll_offset + max_visible).min(matches.len());

        for row in 0..max_visible {
            execute!(stdout, ct::Clear(ct::ClearType::CurrentLine))?;
            let match_idx = scroll_offset + row;
            if match_idx < end {
                let (orig_idx, _score) = matches[match_idx];
                let label = items[orig_idx].as_ref();
                let line = if match_idx == selected {
                    format!("  {} {}", t::accent("❯"), t::accent_bright(label))
                } else {
                    format!("    {}", t::muted(label))
                };
                write!(stdout, "{}\r\n", line)?;
            } else {
                write!(stdout, "\r\n")?;
            }
        }

        // Hint line
        execute!(stdout, ct::Clear(ct::ClearType::CurrentLine))?;
        if matches.is_empty() {
            write!(stdout, "  {}\r\n", t::warn("No matches"))?;
        } else {
            write!(
                stdout,
                "  {}\r\n",
                t::muted(&format!(
                    "{}/{} · type to filter · ↑↓ navigate · Enter select · Esc cancel",
                    selected + 1,
                    matches.len(),
                )),
            )?;
        }
        stdout.flush()
    };

    ct::enable_raw_mode()?;

    let result = (|| -> Result<Option<usize>> {
        let mut query = String::new();
        let mut matches = compute_matches(&query, &mut matcher);
        let mut selected: usize = 0;
        let mut scroll_offset: usize = 0;

        // Initial draw
        draw(&mut stdout, &query, &matches, selected, scroll_offset)?;

        loop {
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        anyhow::bail!("Interrupted");
                    }
                    KeyCode::Esc => {
                        return Ok(None);
                    }
                    KeyCode::Enter => {
                        if !matches.is_empty() {
                            let (orig_idx, _) = matches[selected];
                            return Ok(Some(orig_idx));
                        }
                    }
                    KeyCode::Up => {
                        if selected > 0 {
                            selected -= 1;
                            if selected < scroll_offset {
                                scroll_offset = selected;
                            }
                        }
                    }
                    KeyCode::Down => {
                        if !matches.is_empty() && selected + 1 < matches.len() {
                            selected += 1;
                            if selected >= scroll_offset + max_visible {
                                scroll_offset = selected - max_visible + 1;
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        if query.pop().is_some() {
                            matches = compute_matches(&query, &mut matcher);
                            selected = 0;
                            scroll_offset = 0;
                        }
                    }
                    KeyCode::Char(c) => {
                        query.push(c);
                        matches = compute_matches(&query, &mut matcher);
                        selected = 0;
                        scroll_offset = 0;
                    }
                    _ => continue,
                }

                draw(&mut stdout, &query, &matches, selected, scroll_offset)?;
            }
        }
    })();

    // Always restore cooked mode.
    let _ = ct::disable_raw_mode();

    result
}

/// Interactive arrow-key selector.
///
/// Renders a scrollable list of items with a `❯` marker and handles
/// Up / Down / Home / End / Enter / Esc / Ctrl-C navigation in raw mode.
/// Returns the selected index, or `None` if the user pressed Esc.
pub(crate) fn arrow_select(items: &[impl AsRef<str>], heading_text: &str) -> Result<Option<usize>> {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
    use crossterm::{cursor, execute, terminal as ct};

    if items.is_empty() {
        return Ok(None);
    }

    let mut selected: usize = 0;
    let max_visible: usize = 14;

    // Print the heading (above the interactive region)
    println!("{}", t::heading(heading_text));
    println!();

    let visible_count = items.len().min(max_visible);
    // We render `visible_count` lines for the items + 1 hint line.
    let draw_height = visible_count + 1;

    // Pre-allocate the lines so we can overwrite them.
    for _ in 0..draw_height {
        println!();
    }

    let mut stdout = io::stdout();

    // Helper: draw the visible slice starting at `scroll_offset`.
    let draw = |stdout: &mut io::Stdout, selected: usize, scroll_offset: usize| -> io::Result<()> {
        // Move cursor up to the first item line.
        execute!(stdout, cursor::MoveUp(draw_height as u16))?;

        let end = (scroll_offset + max_visible).min(items.len());
        for (i, item) in items.iter().enumerate().take(end).skip(scroll_offset) {
            let label = item.as_ref();
            let line = if i == selected {
                format!("  {} {}", t::accent("❯"), t::accent_bright(label),)
            } else {
                format!("    {}", t::muted(label))
            };
            // Clear the line, print with \r\n (raw mode needs explicit CR).
            execute!(stdout, ct::Clear(ct::ClearType::CurrentLine))?;
            write!(stdout, "{}\r\n", line)?;
        }

        // Hint line
        execute!(stdout, ct::Clear(ct::ClearType::CurrentLine))?;
        if items.len() > max_visible {
            write!(
                stdout,
                "  {}\r\n",
                t::muted(&format!(
                    "{}/{} · ↑↓ navigate · Enter select · Esc cancel",
                    selected + 1,
                    items.len(),
                )),
            )?;
        } else {
            write!(
                stdout,
                "  {}\r\n",
                t::muted("↑↓ navigate · Enter select · Esc cancel")
            )?;
        }
        stdout.flush()
    };

    ct::enable_raw_mode()?;

    let result = (|| -> Result<Option<usize>> {
        let mut scroll_offset: usize = 0;

        // Initial draw
        draw(&mut stdout, selected, scroll_offset)?;

        loop {
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        anyhow::bail!("Interrupted");
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {
                        return Ok(None);
                    }
                    KeyCode::Enter => {
                        return Ok(Some(selected));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if selected > 0 {
                            selected -= 1;
                            if selected < scroll_offset {
                                scroll_offset = selected;
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if selected + 1 < items.len() {
                            selected += 1;
                            if selected >= scroll_offset + max_visible {
                                scroll_offset = selected - max_visible + 1;
                            }
                        }
                    }
                    KeyCode::Home => {
                        selected = 0;
                        scroll_offset = 0;
                    }
                    KeyCode::End => {
                        selected = items.len().saturating_sub(1);
                        scroll_offset = items.len().saturating_sub(max_visible);
                    }
                    _ => continue,
                }

                draw(&mut stdout, selected, scroll_offset)?;
            }
        }
    })();

    // Always restore cooked mode.
    let _ = ct::disable_raw_mode();

    result
}

pub(crate) fn prompt_secret(_reader: &mut impl BufRead, prompt: &str) -> Result<String> {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

    print!("{}", prompt);
    io::stdout().flush()?;

    // Enable raw mode to suppress echo and line buffering.
    terminal::enable_raw_mode()?;

    let result = (|| -> Result<String> {
        let mut buf = String::new();
        loop {
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Enter => break,
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        anyhow::bail!("Interrupted");
                    }
                    KeyCode::Backspace => {
                        buf.pop();
                    }
                    KeyCode::Char(c) => {
                        buf.push(c);
                    }
                    _ => {}
                }
            }
        }
        Ok(buf)
    })();

    // Always restore cooked mode, even on error.
    let _ = terminal::disable_raw_mode();
    // Print newline since Enter was consumed without echo.
    println!();

    result
}
