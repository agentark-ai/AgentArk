# Swarm, agents, and delegation

Primary surface: top-level `Agents` page, backed by the swarm system.

Use this area when the question is about specialist agents, live agent roster, or delegation behavior.

How it works:

1. AgentArk can delegate parts of complex work to specialist agents.
2. The live roster appears in the `Agents` page.
3. Busy/idle state helps show whether specialists are actively working.
4. Swarm config controls which specialists exist and how they are provisioned.

What to tell users:

- In normal use, users can ask in chat for monitoring, escalation, deep research, or multi-step execution and AgentArk decides when swarm delegation is appropriate.
- The `Agents` page is for visibility and management, not the only way to trigger delegation.
- Updating swarm configuration may require restart before new saved config fully activates.

Verification:

- `Agents` should show registered specialist agents when swarm is configured.
- Swarm status should report enabled and show live counts.
- During delegated work, agent status should move away from fully idle.

Common issues:

- The user expects swarm to work, but the instance has no configured specialist agents.
- A specialist was saved in config, but the process has not restarted to fully apply the new roster.
- The user expects every task to fan out; many tasks are intentionally handled by the main agent alone.
