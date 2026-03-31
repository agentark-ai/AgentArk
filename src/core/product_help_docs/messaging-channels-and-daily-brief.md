# Messaging channels and daily brief

Path: `Settings > Integrations > Messaging Channels`.

Use this area to configure delivery channels and channel-specific credentials.

What is here:

- Telegram
- Slack
- Discord
- Matrix
- Teams
- WhatsApp
- Daily Brief scheduling and delivery channel selection

Recommended channel setup flow:

1. Open `Settings > Integrations > Messaging Channels`.
2. Enable the channel you want.
3. Fill the required token, webhook, room, team, or recipient fields for that channel.
4. Save settings.
5. Check the channel status card until it changes from `Not configured` to a ready state.

Channel-specific notes:

- Telegram usually needs a bot token and a valid user or recipient context.
- Slack usually needs a bot token, signing secret, and delivery target.
- Discord can use a bot token or webhook destination, but it still needs a live destination.
- Matrix needs an access token and room binding.
- Teams needs an access token and a valid reply target.
- WhatsApp needs bridge/cloud settings and usually a recipient interaction before delivery is considered ready.

Daily Brief:

- The `Daily Brief` section lives in the same Messaging Channels area.
- Pick the time, timezone-aware delivery channel, and enable it.
- If the chosen channel is not fully configured, AgentArk should warn that delivery is not ready.

Verification:

- The channel card should read `Ready to deliver`, not `Needs target` or `Not configured`.
- A Daily Brief should only be enabled after the selected delivery channel is ready.

Common issues:

- Credentials saved but no recipient or room selected.
- Daily Brief enabled on a channel that is connected but has no delivery target.
- WhatsApp/Telegram configured but never contacted by the user, so no usable delivery destination exists yet.
