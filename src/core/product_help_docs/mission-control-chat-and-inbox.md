# Mission Control, chat, and approvals

Primary entry points:

- `Mission Control` is the landing overview. Use it for suggested next actions, current priorities, approvals, and operational pulse.
- `Chat` is the main execution workspace. Use it when you want AgentArk to answer, plan, call tools, or run multi-step work.
- Approval, warning, and follow-up items that need a user decision appear in `Mission Control` under its attention surfaces. Older references to `Inbox` now map there.

How to use them:

1. Start in `Chat` when you want the agent to do work immediately.
2. Use `Mission Control` when you want a quick overview of what is waiting, what is failing, or what the system suggests next.
3. Return to `Mission Control` when the agent is waiting for approval or has surfaced something that needs review.

What belongs where:

- Chat: asking questions, starting tasks, deep research, browser work, drafts, coding, and tool execution.
- Mission Control: summary cards, suggested actions, approvals, alerts, review items, and operational shortcuts.

Verification:

- If a task needs approval, it should appear in `Mission Control` and in the relevant `Tasks` flow.
- If a run completed, `Trace` should show what happened and `Mission Control` should stop showing it as pending.
- If the user asks "where do I talk to the agent?", the correct answer is `Chat`.
