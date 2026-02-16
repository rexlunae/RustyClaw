# Skills Guide

Skills are loaded from `SKILL.md` files and can be gated by runtime requirements.

Key behavior:

- Frontmatter parsing and instruction extraction
- Gate checks (`bins`, `anyBins`, `env`, `config`, `os`)
- Workspace-aware loading and precedence rules
- Skill tool support for listing, searching, installing, and metadata

Implementation:

- Skill manager: `src/skills.rs`
- Skill-related tools: `src/tools/skills_tools.rs`

CLI:

```bash
rustyclaw skills list
rustyclaw skills
```
