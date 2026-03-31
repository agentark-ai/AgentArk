# Plugins, webhooks, and custom APIs

Paths:

- `Settings > Integrations > Webhooks & APIs`
- `Settings > Integrations > Plugins`

What belongs where:

- `Webhooks & APIs`: incoming webhook sources, webhook events, and imported custom APIs.
- `Plugins`: third-party plugin SDK integrations and their subscribed platform events.

How to use `Webhooks & APIs`:

1. Create or edit a webhook source.
2. Save the webhook configuration.
3. Use the built-in test action to verify the source.
4. Review incoming events and downstream execution in `Trace` or `Tasks`.

How to use `Custom APIs`:

1. Import or configure the custom API in the same `Webhooks & APIs` area.
2. Confirm it is enabled.
3. Use it from chat or from flows that depend on that API.

How to use `Plugins`:

1. Open `Settings > Integrations > Plugins`.
2. Install or edit the plugin.
3. Enable only the platform events the plugin should receive.
4. Save once so plugin actions and test controls become available.

Important behavior:

- Plugins only receive the platform events you explicitly enable.
- Webhooks are ingress. They create or trigger downstream work; they are not the execution history themselves.
- Imported custom APIs appear alongside other integration surfaces, but they are distinct from prebuilt connectors.

Verification:

- A webhook source should pass its test action.
- A custom API should appear as enabled after import.
- A plugin should appear in the installed plugin list and expose the expected actions or event subscriptions.
