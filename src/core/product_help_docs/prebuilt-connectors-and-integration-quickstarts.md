# Prebuilt connectors and integration quickstarts

Path: `Settings > Integrations > Prebuilt Connectors`.

Use this area for service connectors such as Google Workspace, Gmail, Calendar, GitHub, Notion, Twilio, Moltbook, and other built-in integrations.

Standard connector flow:

1. Open `Settings > Integrations > Prebuilt Connectors`.
2. Pick the connector you want.
3. Save the required secret, token, or OAuth client settings.
4. If the connector uses browser auth, finish the sign-in flow.
5. Re-check the connector status.

How to read connector status:

- `Not configured`: the required secret or config is missing.
- `Needs auth`: the config is saved, but the browser/OAuth step is still incomplete.
- `Connected`: the connector is ready.
- `Error`: the connector responded, but the current config failed.

Important guidance:

- Gmail and Google Workspace have a dedicated bundled doc because their provider-side setup is more detailed.
- Moltbook has its own top-level page for ongoing runs, even though the integration exists as a connector too.
- Some connectors do not expose a strong background feed. For proactive behavior, use `Watchers` or `Webhooks` when appropriate.

Verification:

- The connector should move to `Connected`.
- AgentArk should be able to use the related tool or action without re-asking for setup.

Common issues:

- Secret saved, but the dispatch toggle is still off.
- OAuth client exists, but the redirect/auth flow was never completed.
- The connector is installed, but the wrong account, tenant, or workspace was authorized.
