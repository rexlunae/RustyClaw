//! Build script: regenerate the application icons from the project logo.
//!
//! Renders `../../logo.svg` with resvg (pure Rust — no rsvg-convert,
//! iconutil, or ImageMagick needed) into:
//!
//!   - `$OUT_DIR/icon-256.png` — embedded into the binary as the window icon
//!   - `icons/*.png`, `icons/icon.icns`, `icons/icon.ico` — the icon set
//!     used by `dx bundle` (gitignored: generated, never committed; written
//!     best-effort so a read-only source tree warns instead of failing)
//!
//! Re-runs only when the logo (or this script) changes, so the icon set
//! can never go stale relative to the logo.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use resvg::{tiny_skia, usvg};

const LOGO: &str = "../../logo.svg";

/// All sizes rendered from the logo (superset of every consumer's needs).
const SIZES: [u32; 7] = [16, 32, 64, 128, 256, 512, 1024];

/// Sizes packed into the Windows `.ico`.
const ICO_SIZES: [u32; 5] = [16, 32, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={LOGO}");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    let embedded = out_dir.join("icon-256.png");

    // The icons are generated, not committed, so there is no fallback:
    // a failure here must fail the build (the embedded window icon is
    // include_bytes!'d from OUT_DIR).
    if let Err(e) = generate_icons(&embedded) {
        panic!("failed to generate app icons from {LOGO}: {e}");
    }
}

fn generate_icons(embedded: &Path) -> Result<(), String> {
    let data = fs::read(LOGO).map_err(|e| format!("read {LOGO}: {e}"))?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .map_err(|e| format!("parse {LOGO}: {e}"))?;

    // Render every size once.
    let mut rendered: Vec<(u32, tiny_skia::Pixmap)> = Vec::with_capacity(SIZES.len());
    for size in SIZES {
        let mut pixmap = tiny_skia::Pixmap::new(size, size)
            .ok_or_else(|| format!("allocate {size}px pixmap"))?;
        let sx = size as f32 / tree.size().width();
        let sy = size as f32 / tree.size().height();
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(sx, sy),
            &mut pixmap.as_mut(),
        );
        rendered.push((size, pixmap));
    }

    let png_of = |size: u32| -> Result<Vec<u8>, String> {
        rendered
            .iter()
            .find(|(s, _)| *s == size)
            .expect("size in SIZES")
            .1
            .encode_png()
            .map_err(|e| format!("encode {size}px png: {e}"))
    };

    // The embedded window icon is the only mandatory artifact.
    fs::write(embedded, png_of(256)?).map_err(|e| format!("write embedded icon: {e}"))?;

    // Refresh the committed icon set best-effort: a read-only source tree
    // (e.g. a vendored or published crate) must not fail the build.
    let refresh = || -> Result<(), String> {
        let icons = Path::new("icons");
        fs::create_dir_all(icons).map_err(|e| e.to_string())?;
        for (name, size) in [
            ("32x32.png", 32),
            ("128x128.png", 128),
            ("128x128@2x.png", 256),
            ("icon-256.png", 256),
        ] {
            fs::write(icons.join(name), png_of(size)?).map_err(|e| e.to_string())?;
        }
        write_icns(&icons.join("icon.icns"), &rendered)?;
        let ico_pngs: Vec<(u32, Vec<u8>)> = ICO_SIZES
            .iter()
            .map(|&s| png_of(s).map(|png| (s, png)))
            .collect::<Result<_, _>>()?;
        write_ico(&icons.join("icon.ico"), &ico_pngs).map_err(|e| e.to_string())?;
        Ok(())
    };
    if let Err(e) = refresh() {
        println!("cargo:warning=could not refresh committed icons/ set ({e})");
    }

    Ok(())
}

/// Encode all rendered sizes into an Apple `.icns` icon family.
fn write_icns(path: &Path, rendered: &[(u32, tiny_skia::Pixmap)]) -> Result<(), String> {
    let mut family = icns::IconFamily::new();
    for (size, pixmap) in rendered {
        // tiny-skia stores premultiplied RGBA; icns wants straight alpha.
        let mut rgba = Vec::with_capacity((size * size * 4) as usize);
        for px in pixmap.pixels() {
            let c = px.demultiply();
            rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        let image = icns::Image::from_data(icns::PixelFormat::RGBA, *size, *size, rgba)
            .map_err(|e| format!("icns image {size}px: {e}"))?;
        family
            .add_icon(&image)
            .map_err(|e| format!("icns add {size}px: {e}"))?;
    }
    let file = fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    family
        .write(io::BufWriter::new(file))
        .map_err(|e| format!("write icns: {e}"))
}

/// Pack PNG frames into a Windows `.ico` container (PNG-in-ICO, supported
/// since Windows Vista).
fn write_ico(path: &Path, entries: &[(u32, Vec<u8>)]) -> io::Result<()> {
    let mut buf = Vec::new();
    // ICONDIR: reserved, type (1 = icon), count.
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    // ICONDIRENTRY per frame; image data follows the directory.
    let mut offset = (6 + 16 * entries.len()) as u32;
    for (size, png) in entries {
        let dim = if *size >= 256 { 0u8 } else { *size as u8 }; // 0 encodes 256
        buf.extend_from_slice(&[dim, dim, 0, 0]);
        buf.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        buf.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        buf.extend_from_slice(&(png.len() as u32).to_le_bytes());
        buf.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in entries {
        buf.extend_from_slice(png);
    }
    fs::write(path, buf)
}
