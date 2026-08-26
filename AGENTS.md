# Conduit agent rules

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
