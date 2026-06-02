# agent.md — working in this repo

A short orientation for an agent working **on** Puffer. It deliberately does not restate policy: `AGENTS.md` is the source of truth for the mission, crate roster, hard constraints, slash-command surface, auth, Anthropic-compatibility, and the resource/session models. This file is just the day-to-day surface and the non-obvious gotchas that `AGENTS.md` doesn't spell out.

## Read order

1. `soul.md` — who Puffer is (identity, values, voice).
2. `AGENTS.md` — repo policy, constraints, and scope. **Authoritative**; if this file ever disagrees, `AGENTS.md` wins.
3. This file — commands and gotchas.

## Commands

```
cargo build --workspace
cargo test --workspace          # the gate — keep it green (AGENTS.md > Working Style)
cargo test -p <crate>           # scope to the crate you touched first, then widen
cargo fmt --all
cargo clippy --workspace --all-targets
```

There is no `justfile`/`Makefile` — use cargo directly. Test narrow first, then `--workspace` before calling it done; wire tests in the same change that adds the behavior.

## Gotchas not covered in AGENTS.md

- **System-prompt drift trap.** The runtime system prompt is `resources/prompts/system-base.yaml`, assembled in `crates/puffer-core/runtime/system_prompt.rs`. That template body is **also mirrored verbatim** as the `SYSTEM_PROMPT_TEMPLATE` const in the same module (used as the fallback when the YAML fails to load). If you edit the YAML body, mirror the const too, or they silently diverge.
- **Project docs are loaded by name, and not these two.** `load_memory_prompt` (in `system_prompt.rs`) injects `AGENTS.md` for the OpenAI provider and `CLAUDE.md` for the others. `soul.md` and `agent.md` are **not** read at runtime — they are contributor docs. To make the soul load-bearing, author it as `resources/prompts/soul.yaml` and chain it in via the `chained_from` field (and respect the drift trap above).
- **Personas and task prompts** are declarative: sub-agents in `resources/agents/*.yaml`, prompts in `resources/prompts/*.yaml`. Prompts support `provider_override` / `model_override` / `chained_from`.

Everything else — the hard constraints, the `specs/<component>/NN.md` convention, commit/worktree rules, Anthropic-path fidelity — is in `AGENTS.md`. Follow it.
