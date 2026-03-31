# Settings and navigation map

Use these paths when explaining where users configure things in AgentArk.

Primary areas:

- Mission Control / Chat
  Use these for the primary operator workflow: overview, execution, approvals, and alerts.

- Settings > Models
  Use this for LLM/provider setup, API keys for model providers, and model behavior configuration.

- Settings > Media
  Use this for image/video provider API keys and default media-generation providers.

- Settings > Integrations > Messaging Channels
  Use this for channel delivery surfaces such as Telegram, WhatsApp, Slack, Discord, Matrix, and Teams.

- Settings > Integrations > Prebuilt Connectors
  Use this for external services such as Google Workspace, Gmail, Calendar, GitHub, Notion, Twilio, and other connectors.

- Settings > Integrations > Webhooks & APIs
  Use this for webhook/API-facing integration setup.

- Settings > Integrations > Plugins
  Use this for plugin-backed integrations.

- Settings > Knowledge > Memory
  Use this for structured memory, reusable knowledge-base items, preferences, and user data. The reusable KB items live under the `Knowledge` tab inside this page.

- Library > Documents
  Use this for uploaded files and indexed document context.

- Moltbook
  Use this top-level page for Moltbook API key setup, status, run-now controls, and activity logs.

- Tasks
  Use this for scheduled and one-off tasks.

- Watchers
  Use this for monitor/poll-until workflows.

- Apps
  Use this for generated or managed apps, deployment, and app status.

- Goals / Agents / ArkPulse
  Use these for long-running outcomes, specialist agents/swarm visibility, and operational health guidance.

- Trace / Analytics
  Use these when the user wants to inspect what the agent did, how it failed, or how it performed.

- Settings > Security / Advanced / Evolution
  Use these for security controls, advanced operator settings, and self-learning or deploy-guard behavior.

Navigation guidance:

- If the user asks where to add credentials, send them to Settings.
- If they ask where the main conversation happens, send them to Chat.
- If they ask where approvals go, send them to Mission Control.
- If they ask where Google Workspace or another connector is configured, send them to Settings > Integrations > Prebuilt Connectors.
- If they ask where reusable notes or KB entries live, send them to Settings > Knowledge > Memory > Knowledge.
- If they ask where uploaded files live, send them to Library > Documents.
- If they ask where to configure image or video generation providers, send them to Settings > Media.
- If they ask how to run or configure Moltbook, send them to the top-level Moltbook page.
- If they ask where an automation runs later, send them to Tasks or Watchers depending on whether it is scheduled work or condition-based monitoring.
- If they ask how to inspect behavior, send them to Trace or Analytics.
- If they ask about specialist agents or delegation, send them to Agents.
- If they ask about health findings and guided remediation, send them to ArkPulse.

When the user asks a "where do I configure X?" question, answer with the exact path first, then the steps.
