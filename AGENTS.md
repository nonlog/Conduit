# Conduit agent rules

## Product priority

- Conduit exists because Link to Windows consumed too much CPU on the phone and caused lag, heat,
  and battery drain. Preserving low idle CPU, low wakeup frequency, low radio activity, and low
  background memory/thread/socket cost is the primary product constraint.
- Do not add periodic polling, recurring throughput tests, background benchmarks, keep-awake loops,
  or continuous scoring work merely to make routing or UX "smarter". Prefer event-driven callbacks,
  blocked I/O, bounded one-shot work, passive measurements from traffic that already exists, and
  work shifted to the plugged-in Windows side when possible.
- A feature that materially increases Android idle CPU/radio wakeups must justify that cost explicitly
  and should be rejected or redesigned when an event-driven alternative exists.

## Continuity / handoff

- `docs/CONDUIT_HANDOFF.md` is the live resume checkpoint for this repository.
- Update it during development after any major implementation milestone, production/runtime
  deployment or configuration change, important verification result, newly established root cause,
  or change to the recommended next step. Do not wait until a conversation is nearly out of context.
- Keep handoff facts concrete: current branch/HEAD, dirty-worktree caveats, deployed/installed
  state, test evidence, known failures, machine-runtime dependencies, and the safest next action.
- Never copy passwords, API keys, tokens, private keys, or other secrets into the handoff.
- Before ending a substantive development session, verify the handoff still describes the current
  repository/runtime state closely enough that another agent can continue without reconstructing
  the previous chat.

## Git attribution

- Commits created by an agent must use `Codex <codex@openai.com>` as both author and committer.
- Do not rewrite historical commit authorship unless the user explicitly asks.
- Do not push unless the user explicitly requests it in the current context.
