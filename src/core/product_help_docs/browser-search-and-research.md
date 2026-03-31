# Browser automation, search, and research

These capabilities are primarily chat-native rather than settings-first workflows.

What they do:

- `Web search`: quick source lookup.
- `Research`: deeper, slower, source-backed investigation.
- `Browser automation`: website navigation, form filling, reading pages, screenshots, and login-like workflows with user assist when needed.

How to use them:

1. Ask in `Chat` for online research or browser work.
2. Turn on the `Research` toggle in chat when the user wants a deeper, source-backed answer.
3. Ask for browser actions in plain language when the task needs real website interaction.
4. Use `Trace` afterward to inspect what happened.

Important behavior:

- Research is not the same as a simple web search. It is the heavier path.
- Browser automation is session-based and can pause for user help on CAPTCHAs, 2FA, or ambiguous pages.
- If the user asks for a provider-side setup flow that drifts over time, the agent should keep AgentArk-specific steps from local docs and verify the external console steps with official web sources.

Verification:

- A research run should cite or reflect source-backed findings.
- A browser run should leave trace evidence of navigation, reading, screenshots, or interaction steps.

Common issues:

- The user wants a current answer but asks without enabling research or web use.
- The browser reached a human checkpoint and needs user input before it can continue.
- The user expects a settings page for everything; browser and research workflows often begin directly in chat.
