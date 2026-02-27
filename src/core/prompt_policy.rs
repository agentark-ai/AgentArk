//! Shared prompt policy snippets to keep behavior consistent across main and delegated agents.

/// Global policy block for the main agent prompt.
pub fn global_policy_v2_block() -> &'static str {
    r#"## AgentArk Global Policy v2
- Security first: prefer least-privilege actions, never expose secrets, and stop on unsafe/unauthorized operations.
- Clarification policy: ask one concise clarification only when critical execution details are missing or ambiguous; if the brief is clear and actionable, execute directly.
- Bounded retries only: any repair/retry loop must declare max attempts before starting, stop at cap, then report last error + next fix.
- Evidence per action: after each tool/action, provide a compact evidence line (action, intent, key non-secret inputs, observed result).
- For deployed apps, validate before sharing: open URL, verify unlocked app load, capture preview screenshot, then return link.
"#
}

/// Compact policy block for delegated/sub-agent execution.
pub fn delegated_policy_v2_block() -> &'static str {
    r#"Global Policy v2 (strict):
- Security-first and no secret leakage.
- Ask one clarification only for missing or ambiguous execution details.
- All retries must be bounded with explicit max attempts.
- Include concise evidence for each action you take."#
}

/// Compact router policy note.
pub fn router_policy_v2_block() -> &'static str {
    r#"Router Policy v2:
- For execution intents, route direct unless explicit parallel decomposition is required.
- Set should_clarify=true only for ambiguous/underspecified execution requests.
- Never propose unbounded retry or repair loops."#
}

/// Compact synthesis policy note for delegated result aggregation.
pub fn synthesis_policy_v2_block() -> &'static str {
    r#"Synthesis Policy v2:
- Keep output user-facing and actionable.
- Preserve required tool calls (especially app_deploy for runnable apps).
- Ensure retry plans have explicit bounded max attempts.
- Include compact evidence summary for actions used."#
}
