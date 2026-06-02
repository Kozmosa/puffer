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

The dev loop is plain cargo. The root `Makefile` only wraps release/packaging (`scripts/release.sh`), not build/test. Test narrow first, then `--workspace`.

## Gotchas not covered in AGENTS.md

- **System-prompt drift trap.** The runtime system prompt is `resources/prompts/system-base.yaml`, assembled in `crates/puffer-core/runtime/system_prompt.rs`. The same body is *also* hand-mirrored as the `SYSTEM_PROMPT_TEMPLATE` const in that module (used when the `system-base` prompt resource isn't found/registered). It is meant to be kept in sync but has **already drifted** — the const carries non-ASCII em-dashes and a "tool calls are visible in the terminal" sentence that directly contradicts the YAML's "users can't see most tool calls." If you edit one, fix the other.
- **Project docs are loaded by name, and not these two.** `load_memory_prompt` (in `system_prompt.rs`) loads `CLAUDE.md` for all providers; the OpenAI provider prefers `AGENTS.md` and falls back to `CLAUDE.md`. `soul.md` and `agent.md` are **not** read at runtime — they are contributor docs. (The persona could be made load-bearing via a chained `resources/prompts/soul.yaml`, but that is not currently wired.)
- **Personas and task prompts** are declarative: sub-agents in `resources/agents/*.yaml`, prompts in `resources/prompts/*.yaml`. Prompts support `provider_override` / `model_override` / `chained_from`.

Everything else — the hard constraints, the `specs/<component>/NN.md` convention, commit/worktree rules, Anthropic-path fidelity — is in `AGENTS.md`. Follow it.
