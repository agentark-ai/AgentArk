# Add Gmail access through Google Workspace

Recommended path: connect Google Workspace once, then use its Gmail and Calendar access from the same Google sign-in.

Steps inside AgentArk:

1. Open Settings > Integrations > Prebuilt Connectors.
2. Find Google Workspace in the connector list.
3. Enter the Google OAuth Client ID and Google OAuth Client Secret for this AgentArk instance.
4. In Workspace Bundles, include at least `gmail`. Add `calendar` too if you also want calendar support. Save the setup.
5. Click Continue with Google / Connect so AgentArk opens the browser sign-in flow.
6. Sign in with the Google account you want AgentArk to use and grant the requested scopes.
7. Return to AgentArk and check that Google Workspace shows connected. If Gmail is the main goal, verify the Gmail-related status is healthy too.

Steps outside AgentArk in Google Cloud:

1. Open Google Cloud Console and create or select a project.
2. Configure the OAuth consent screen.
3. If the app is still in testing, add yourself as a test user.
4. Create an OAuth client and copy the client ID and client secret.
5. Add this redirect URI exactly:

   `http://localhost:8990/oauth/callback`

6. Enable the Google APIs you want AgentArk to use. For Gmail access, enable Gmail API. If you also want broader Workspace support, enable the APIs that match your selected bundles such as Google Calendar API, Drive API, Docs API, Sheets API, Google Chat API, and Admin SDK.

Verification:

- In Settings > Integrations > Prebuilt Connectors, Google Workspace should no longer say not configured or needs auth.
- A connection test should pass.
- AgentArk should be able to list Gmail or use the Google Workspace helper actions without asking for setup again.

Common issues:

- Redirect URI mismatch: the URI in Google Cloud must exactly match `http://localhost:8990/oauth/callback`.
- App still in testing without your account added: add yourself as a test user.
- Missing Gmail API: enable Gmail API in Google Cloud.
- Wrong bundles selected: update the bundle list in Google Workspace settings and reconnect if needed.

If the user asks specifically for "Gmail access", prefer this Google Workspace path unless they explicitly want the separate legacy Gmail-only connector.
