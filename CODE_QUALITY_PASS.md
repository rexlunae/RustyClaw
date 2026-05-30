# Code Quality Pass — Status Report

_Date: 2026-05-30_

Audit of RustyClaw code quality plus implementation of the priority fixes.

---

## 1. Audit findings (priority order)

1. **Lint gates disabled in CI** — `Cargo.toml` declares `clippy::all = "deny"`, but
   `.github/workflows/ci.yml` ran `cargo fmt`/`cargo clippy` with
   `continue-on-error: true` ("advisory for now"), so failures never blocked merges.
   The tree had drifted and clippy did not pass.
2. **`provider_registry.rs` was dead code** — 975-line `genai`-backed parallel provider
   system with zero references in the workspace, and the sole consumer of the heavy
   `genai` dependency. An abandoned migration.
3. **Files too long** — `gateway/mod.rs` (3,364; one ~2,700-line `run_gateway` fn),
   `cli/main.rs` (3,173), `tools/mod.rs` (3,034), `tui/app/tui_component/mod.rs` (2,594),
   `tui/onboard.rs` (2,137), `gateway/providers.rs` (2,071).
4. **Errors swallowed on state-persisting writes** — many `let _ = config.save(None)` /
   `fs::write` sites silently discard failures, risking silent data loss.
5. **Three markdown implementations** (core/tui/desktop) — mostly justified.
6. **Functional TODOs** representing missing behavior.
7. **25 `#[allow(dead_code)]` sites** worth a sweep.

Positives: no `unwrap`/`expect`/`panic!` in non-test paths; strong module docs; sensible
crate boundaries; good security posture.

---

## 2. Changes made — all verified ✅

### 2a. Lint gate enforced (item #1) — ✅ DONE
- Removed `continue-on-error: true` from the `fmt` and `clippy` steps in
  `.github/workflows/ci.yml`. The lint job now blocks merges.
- The clippy step runs `cargo clippy --workspace --all-targets -- -D warnings`
  (default features) with a comment explaining why `--all-features`/`--features full`
  are excluded (see §3).
- **Fixed every clippy error this surfaced** across the workspace:
  - **tokenjuice**: `redundant_closure`, `if_same_then_else`, `single_match`/`collapsible_if`.
  - **rustyclaw-core** (13 errors, only visible under `--features full`):
    `field_reassign_with_default` (×2), unstable `floor_char_boundary` past MSRV 1.86,
    `io::Error::other` (×4), empty-line-after-doc, `if let Err…return Err` → `?` (×2),
    `redundant_closure`, `len_without_is_empty`, `needless_range_loop`,
    `if_same_then_else`, derivable `Default`, `needless_option_as_deref`.
  - **rustyclaw-view**: `clone_on_copy`, `single_match`, and a broken integration test
    (`MessageBubbleData` literal missing the `collapsed` field — E0063 compile error).
  - **rustyclaw-desktop** (~12): `collapsible_if` (×6), `redundant_closure` (×2),
    doc-list overindent (×2), `manual_contains`, `suspicious_else_formatting` (rsx macro
    interaction, fixed by hoisting the `if/else` out of the macro), and a dead
    `SecretsCommand::Store` variant (marked `#[allow(dead_code)]` with a note that it's
    reserved for the not-yet-wired add-secret flow).

### 2b. Dead code / dependency removal (item #2) — ✅ DONE
- Deleted `crates/rustyclaw-core/src/provider_registry.rs` (975 lines).
- Removed `pub mod provider_registry;` from `lib.rs`.
- Removed the `genai = "0.5.3"` dependency from `rustyclaw-core/Cargo.toml`.
  (`genai` may still linger in the gitignored `Cargo.lock` until a regen; cosmetic.)

### 2c. Swallowed write errors (item #4) — ✅ the high-value sites
Replaced silent `let _ = …` with `tracing::warn!` on failure at:
`commands.rs` (×4 `config.save`), `gateway/mod.rs` (×2 `cfg.save`),
`tui/app/app.rs` (×5 `config.save`), `client_prefs.rs` (prefs `fs::write`),
`desktop/components/message.rs` (saved-message `fs::write`).
Left bare best-effort `create_dir_all` calls alone (paired with a checked write).

### 2d. Formatting — ✅ DONE
`cargo fmt --all` applied (reformatted pre-existing drift across many files — large but
required for the new `fmt --check` gate).

---

## 3. Known limitation: `--all-features` cannot compile (pre-existing, upstream)

Two optional features of `rustyclaw-core` do not build, independent of lint work, so the
CI gate deliberately targets default features:
- **`qr`** (`pairing/qr.rs`): `qrcode 0.14` provides no `qrcode::render::Pixel` impl for
  `image 0.25`'s `Luma<u8>`. (`qr` ∉ `full`; only `--all-features` hits it.)
- **`browser`** (in `full`): `chromiumoxide_cdp` fails to build on the current toolchain.

File issues to track these so broader feature coverage can be restored later.

---

## 4. Verification — current pass/fail

| Check | Status |
|---|---|
| `cargo fmt --all -- --check` | ✅ PASS (0 diffs) — exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` (the CI gate) | ✅ PASS (0 errors, 0 warnings) — exit 0 |
| `cargo build --workspace` | ✅ PASS — exit 0 |
| `cargo clippy -p rustyclaw-core --features full --all-targets` | ✅ PASS |
| `cargo test -p tokenjuice` | ✅ PASS (8/8) |
| `cargo clippy --all-targets --all-features` | ❌ cannot build — `qr`+`browser` broken upstream (§3) |
| `rustyclaw-core` lib unit tests | ⚠️ 4 pre-existing env-dependent failures (below), not caused by this pass |

Pre-existing flaky tests in `crates/rustyclaw-core/src/tools/mod.rs`
(`test_tts_returns_media_path`, `test_image_url_detection`, `test_browser_status`,
`tools::browser::tests::test_browser_stub_status`) fail on a clean checkout too
(confirmed via `git stash`). Out of scope; worth a separate fix.

---

## 5. Audit items deferred (not started)

- **#3 — break up oversized files** (`gateway/mod.rs`'s 2,700-line `run_gateway`,
  `cli/main.rs`, `tools/mod.rs`). Biggest maintainability win, largest/riskiest change.
- **#6 — functional TODOs** → tracked issues.
- **#7 — sweep the remaining `#[allow(dead_code)]`** (one, `SecretsCommand::Store`, was
  documented during this pass; the rest remain).
- **#5 — markdown divergence**: judged acceptable; doc note only.

---

## 6. Change set

~82 files in the working tree (uncommitted), including 1 deletion
(`provider_registry.rs`), the targeted logic fixes above, and a large amount of pure
`cargo fmt` reformatting churn. Nothing committed to git.

**Suggestion when committing:** land the `cargo fmt` reformat as its own isolated commit,
separate from the logic/lint changes, for a reviewable history.

### Memory files written
- `project_clippy_gate_enforced.md`, `project_broken_optional_features.md` (indexed in
  `MEMORY.md`).
