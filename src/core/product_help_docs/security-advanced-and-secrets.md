# Security, advanced settings, and secrets

Paths:

- `Settings > Security`
- `Settings > Advanced`

Use `Settings > Security` for:

- master password and secret protection
- security status
- security logs

Use `Settings > Advanced` for:

- lower-level runtime and integration controls
- sender verification and platform-hardening controls
- expert-only settings that are not part of normal onboarding

Secret-handling rules:

1. Prefer settings forms, connector setup, or explicit secret-save flows.
2. Do not ask users to paste secrets into general chat unless the flow explicitly supports secure handling.
3. Treat encrypted secret storage as the source of truth for provider keys, tokens, and connector credentials.

What the agent should explain:

- Secrets are stored encrypted and are handled separately from normal model generation.
- Security logs are for audit/review, not just failures.
- Advanced settings should only be changed when the operator knows why the default is insufficient.

Verification:

- After saving a secret-backed config, the related feature should stop showing `Not configured`.
- Security logs should record meaningful security events.
- If a master password change or protected secret flow succeeded, the instance should still be able to read its encrypted settings.

Common issues:

- A secret exists, but the feature still fails because another required non-secret field is missing.
- Users confuse `Security` logs with `Trace`; `Trace` shows execution, `Security` shows security-relevant events.
- An advanced setting was changed without understanding the effect on public exposure or integration trust boundaries.
