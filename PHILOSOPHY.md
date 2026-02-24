# Philosophy

## Why RustyClaw Exists

Most "AI platforms" are wrappers that lock you into someone else's billing. We've seen too many businesses trapped in vendor ecosystems where prices rise, APIs change, and migration is impossible.

RustyClaw is the opposite: AI infrastructure you control.

## Core Principles

### 🔓 No Vendor Lock-In

Anything a business relies on that can only run on a proprietary cloud is inherently fragile.

- **If your cloud provider doubles prices** → you move
- **If a vendor gets acquired or enshittified** → you migrate  
- **If regulations change** → you relocate

RustyClaw runs on a Raspberry Pi, a $5/month VPS, or enterprise infrastructure. Same binary, same config.

### 💰 No Rent-Seeking

Subscription fees for infrastructure you could own are a tax on your business.

We don't do:
- ❌ Mandatory SaaS dependencies
- ❌ Per-seat licensing for self-hostable software
- ❌ "Free tier" traps that scale into enterprise pricing
- ❌ Proprietary APIs that create switching costs

We do:
- ✅ MIT license, forever
- ✅ Single binary you can copy anywhere
- ✅ Standard protocols over proprietary APIs
- ✅ Local-first architecture (your data stays yours)

### 🔄 Provider Agnostic

Swap LLM providers with a config change. Anthropic today, local Llama tomorrow. No code changes, no migration projects, no vendor negotiations.

```toml
# config.toml - that's it
[model]
provider = "anthropic"  # or "openai", "ollama", "openrouter", ...
model = "claude-sonnet-4-20250514"
```

### 📦 Minimal Dependencies

- **Single binary** — no node_modules, no Docker required, no vendor SDK
- **~15MB RAM** — runs on constrained hardware
- **<50ms startup** — instant, not "warming up"
- **Zero external services** — no mandatory cloud, no telemetry, no phone-home

### 🔐 Own Your Data

- **Local encrypted vault** — secrets on your machine, AES-256
- **File-based memory** — portable, auditable, yours
- **No mandatory cloud** — everything works offline (except the LLM calls)

## The Quote

> "The cloud is just someone else's computer. Make sure you can use anyone's."

## For Contributors

When making design decisions, ask:
1. Does this create vendor dependency?
2. Does this require a specific cloud?
3. Could a user migrate away easily?
4. Are we using standard protocols or inventing proprietary ones?

If something creates lock-in, find another way.

## See Also

- [Persei Labs Values](https://perseilabs.com/#philosophy) — the company behind RustyClaw
- [README.md](README.md) — project overview and quick start
