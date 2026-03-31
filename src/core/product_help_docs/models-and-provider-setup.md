# Models and provider setup

Path: `Settings > Models`.

Use this area to configure the model pool that powers normal chat, coding, research, and fallback behavior.

Recommended setup:

1. Add one `Primary` model slot first. This is the main default model.
2. Optionally add `Fast`, `Code`, `Research`, and `Fallback` slots if you want role-specific routing.
3. Enter the provider, model name, base URL if needed, and the API key or credential for each slot.
4. Leave `Smart routing` on if you want AgentArk to pick between configured slots automatically.
5. Save settings and confirm the slot is enabled.

What the roles mean:

- `Primary`: general default.
- `Fast`: cheaper/faster simple queries.
- `Code`: coding-heavy tasks.
- `Research`: deeper source-backed research flows.
- `Fallback`: used if the preferred slot fails.

Verification:

- `Settings > Models` should show at least one enabled slot.
- The primary slot should be runtime-ready, not just saved.
- A normal chat request should succeed after save.
- If a dedicated research slot exists, source-backed research can use it when the user turns on research mode.

Common issues:

- Saved but unavailable at runtime: the slot exists, but the provider key/base URL is missing or invalid.
- No real primary slot: AgentArk can have saved models, but the main path is weak if no usable primary exists.
- Local model provider unreachable: check the local runtime URL before blaming the model config.
