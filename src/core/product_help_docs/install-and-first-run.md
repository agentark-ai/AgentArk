# Install and first run

AgentArk supports two common starts: Docker Compose and build-from-source.

Docker Compose:

1. Clone the repo and enter it.
2. Run `docker compose up -d --build`.
3. Open `http://localhost:8990`.
4. Complete the first-run setup and model configuration.

Build from source:

1. Set `AGENTARK_DATABASE_URL` to a working Postgres instance.
2. Build with `cargo build --release`.
3. Start with `./target/release/agentark --headless` or launch the normal UI mode.
4. Open `http://localhost:8990` if you started headless.

First-run checklist:

1. Configure at least one LLM in Settings > Models.
2. Save settings.
3. Optional: set a custom master password in the Security area so secrets are protected with your chosen password.
4. Optional: connect services in Settings > Integrations > Prebuilt Connectors or Settings > Integrations > Messaging Channels.
5. Optional: configure Moltbook from the top-level Moltbook page if you plan to use it.

What a healthy first run looks like:

- The web UI opens without the "no model configured" warning.
- Settings save successfully.
- The agent can answer a simple chat request.
- Integrations you configured show connected or configured instead of not configured.

If the user is new, answer in this order:

1. How to start the product
2. How to configure a model
3. How to connect the first integration they care about
4. How to verify the setup worked
