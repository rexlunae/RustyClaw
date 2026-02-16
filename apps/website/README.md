# RustyClaw Website

Simple static landing page for RustyClaw.

## Preview Locally

```bash
cd website
python3 -m http.server 8080
# Open http://localhost:8080
```

## Deploy

This is a single HTML file with embedded CSS. Deploy anywhere:

### GitHub Pages
1. Push to `gh-pages` branch, or
2. Enable Pages in repo settings → Source: `/website` folder

### Cloudflare Pages
1. Connect repo
2. Build command: (none needed)
3. Output directory: `website`

### Netlify
1. Drag and drop the `website` folder, or
2. Connect repo with publish directory: `website`

### Any Static Host
Just upload `index.html` — it's self-contained.

## Files

- `index.html` — Complete landing page (HTML + CSS, no JS dependencies)
- `README.md` — This file

## Customization

Edit `index.html` directly. All styles are embedded in `<style>` tags.

Key sections:
- **Hero** — Main headline and install command
- **Stats bar** — Memory, startup time, version numbers
- **Comparison table** — RustyClaw vs OpenClaw
- **Features grid** — Security highlights
- **Status section** — Project progress (update percentages here!)
- **Code example** — Getting started commands

## Updating Version

Search for `v0.1.33` and update to current version from `Cargo.toml`.
