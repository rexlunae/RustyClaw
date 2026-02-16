# Configuration

RustyClaw reads configuration from:

- `~/.rustyclaw/config.toml`

You can create or update it with:

```bash
rustyclaw configure
```

Minimal example:

```toml
settings_dir = "~/.rustyclaw"

[model]
provider = "openrouter"
model = "gpt-4.1"
base_url = "https://openrouter.ai/api/v1"

[sandbox]
mode = "auto"
```

Reference examples:

- `config.example.toml`
- `docs/SANDBOX.md`
- `docs/SECURITY.md`
