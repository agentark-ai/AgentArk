# AgentArk API

Full interactive API docs available at **http://localhost:8990/docs#/** after starting AgentArk. For installation and configuration, see the [README](README.md).

## Reflect queries

Reflect is cached-read by default. Normal reads should use `GET /reflect`; heavy source scans, embedding, and refresh work are queued separately so the web UI and backend do not hang while a retrospective is prepared.

```bash
# Read the cached weekly reflection for an explicit UTC range.
curl "http://localhost:8990/reflect?period=weekly&from=2026-05-01T00:00:00Z&to=2026-05-08T00:00:00Z"

# Queue a guarded background refresh for that same range.
curl -X POST "http://localhost:8990/reflect/refresh?period=weekly&from=2026-05-01T00:00:00Z&to=2026-05-08T00:00:00Z"

# Shortcut: read cached data and request a refresh in the same call.
curl "http://localhost:8990/reflect?period=monthly&from=2026-05-01T00:00:00Z&to=2026-06-01T00:00:00Z&refresh=1"
```

Supported `period` values are `daily`, `weekly`, and `monthly`. `from` and `to` are RFC3339 timestamps; omit them to use the default window for the selected period. Responses include `clusters`, `source_counts`, `baseline_source_counts`, `embedding_status`, `refresh_status`, `cache_status`, `related_history`, and `unclustered_units`.

Reflect does not store raw per-message chat embeddings. It creates retention-managed `semantic_work_units` from derived summaries and source metadata, embeds those work units, then clusters and compares them across time windows.

Reflect Daily Digest can be enabled in Settings. When enabled, AgentArk prepares a short LLM-written recap after a quiet end-of-day window, stores it in the notification feed, and attempts the selected notification channel. If the structured activity gate finds nothing meaningful, no notification is sent.

## Troubleshooting

<details>
<summary>Settings won't save</summary>

- Check that you have a valid API key for non-Ollama providers
- Ensure the model name is correct
</details>

<details>
<summary>Telegram bot not responding</summary>

- Restart after changing Telegram settings
- Verify your user ID is in Allowed Users
- Check bot token is correct
</details>

<details>
<summary>Data lost after restart</summary>

- Always use Docker volumes - `docker compose` handles this automatically
- If using `docker run`, add `-v agentark-data:/app/data -v agentark-config:/app/config`
</details>

<details>
<summary>Debug logging</summary>

```bash
AGENTARK_DEBUG=true ./scripts/start.sh              # full debug
RUST_LOG=info,agentark=debug ./scripts/start.sh     # agent internals only
```

</details>
