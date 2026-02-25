# PromptGuard for nanobot

A prompt injection defense layer ported from [RustyClaw](https://github.com/rexlunae/RustyClaw) (MIT License).

## What it detects

- **System prompt overrides**: "Ignore previous instructions", "new rules", etc.
- **Role confusion**: "You are now...", "Act as...", "Pretend to be..."
- **Tool call injection**: Malformed JSON, tool_calls injection
- **Secret extraction**: "List all secrets", "dump credentials"
- **Command injection**: Shell metacharacters (`$(`, backticks, `&&`, pipes)
- **Jailbreak attempts**: "DAN mode", "bypass restrictions"

## Installation

Copy `prompt_guard.py` to `nanobot/safety/` directory.

## Usage

```python
from nanobot.safety.prompt_guard import PromptGuard, GuardAction

# Create guard with blocking enabled (sensitivity 0.15 recommended)
guard = PromptGuard(action=GuardAction.BLOCK, sensitivity=0.15)

# Scan user messages before sending to LLM
result = guard.scan(user_message)

if result.blocked:
    return f"Message blocked: {result.message}"
elif result.suspicious:
    log.warning(f"Suspicious patterns: {result.patterns} (score: {result.score})")
```

## Integration with nanobot

In `nanobot/agent/loop.py`, add before the LLM call:

```python
from nanobot.safety.prompt_guard import PromptGuard, GuardAction

# Initialize once
_prompt_guard = PromptGuard(action=GuardAction.WARN, sensitivity=0.15)

async def process_message(user_message: str):
    # Check for prompt injection
    guard_result = _prompt_guard.scan(user_message)
    
    if guard_result.blocked:
        return {"error": guard_result.message}
    
    if guard_result.suspicious:
        # Log but continue (when action=WARN)
        logger.warning(
            "Prompt injection detected",
            patterns=guard_result.patterns,
            score=guard_result.score
        )
    
    # Continue with normal processing...
```

## Configuration

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `action` | GuardAction | WARN | WARN (log only), BLOCK (reject), or SANITIZE (escape) |
| `sensitivity` | float | 0.7 | Score threshold for blocking (0.0-1.0). Lower = more strict |

**Recommended sensitivity**: `0.15` for production blocking.

## Score Breakdown

Each category contributes to the total score (max 1.0 per category, normalized by 6):

| Category | Score | Description |
|----------|-------|-------------|
| system_prompt_override | 1.0 | "Ignore previous instructions" |
| role_confusion | 0.9 | "You are now a different assistant" |
| secret_extraction | 0.95 | "List all secrets" |
| jailbreak_attempt | 0.95 | "Enable DAN mode" |
| tool_call_injection | 0.7-0.8 | Malformed JSON injection |
| command_injection | 0.3 each | Shell metacharacters |

## License

MIT License - Ported from RustyClaw by Persei Labs

## Attribution

If you use this in your project, please credit:

```
Prompt injection detection ported from RustyClaw
https://github.com/rexlunae/RustyClaw
```
