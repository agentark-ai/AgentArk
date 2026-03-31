# App deploy and access guard

App deployment is primarily chat-driven, and deployed apps are managed in the top-level `Apps` page.

Main places:

- Ask in `Chat` to build or deploy an app.
- Use `Apps` to inspect existing deployed apps.
- Use `Settings > Admin > Evolution` to control the default deploy-guard behavior for new app deploys.

Deployment flow:

1. Ask AgentArk in chat to build or deploy the app or repo.
2. Let AgentArk create files or deploy from a repository source.
3. Open the `Apps` page to inspect the deployed result.
4. Use restart, stop, delete, or guard controls from the app card when needed.

Access guard:

- `Access guard` protects a deployed app with an access key.
- The default policy for new deploys can be changed in `Settings > Admin > Evolution`.
- Existing apps can have guard enabled or disabled individually from the `Apps` page.
- If guard is enabled, visitors must provide the access key before viewing the app.

Verification:

- A successful deploy should produce a local URL and sometimes a public URL.
- The app should appear in the `Apps` list with runtime state.
- If guard is enabled, the app card should say guard is enabled and the visitor flow should request the key.

Common issues:

- Deployment succeeded partially, but the runtime failed to start.
- The app exists, but required secrets or config values were missing.
- The user expects a public app, but access guard or exposure settings changed the reachable URL flow.
