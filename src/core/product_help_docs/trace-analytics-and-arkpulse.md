# Trace, analytics, and ArkPulse

Top-level pages:

- `Trace`
- `Analytics`
- `ArkPulse`

What each one is for:

- `Trace`: step-by-step execution history showing what the agent actually did.
- `Analytics`: aggregated usage metrics such as model, channel, and token/cost trends.
- `ArkPulse`: operational health and fix guidance across the instance.

How to use them:

1. Open `Trace` when the user asks "what did the agent do?" or when a run needs debugging.
2. Open `Analytics` when the user asks about usage, volume, model mix, or cost trends.
3. Open `ArkPulse` when the user asks whether the system is healthy or wants guided remediation for operational findings.

ArkPulse specifics:

- ArkPulse can surface findings about runtime state, apps, tunnels, and related health issues.
- Some ArkPulse findings support a direct fix path from the UI.
- Advisory-only findings still need manual action.

Verification:

- A recent run should create a trace entry.
- Analytics should show usage data after real traffic exists.
- ArkPulse should show either findings or a clean recent run state.

Common issues:

- Users look in `Analytics` for a single failed run; the correct place is `Trace`.
- Users look in `Trace` for long-term spend trends; the correct place is `Analytics`.
- Users expect ArkPulse to replace logs; it is a higher-level operational guide, not the raw event stream.
