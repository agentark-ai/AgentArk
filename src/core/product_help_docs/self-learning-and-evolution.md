# Self-learning and evolution

Path: Settings > Admin > Evolution.

Learning and self-evolve are related, but they are not the same thing.

How learning works:

1. Completed or degraded runs are recorded as provisional experience runs.
2. If the user corrects the result within the correction window, that run can be marked corrected instead of being treated as a clean success.
3. Consolidation turns accepted evidence into durable learned memory such as facts, operating constraints, lessons, and procedures.
4. Pattern induction turns repeated successful procedures into learned procedural patterns.
5. Candidate generation creates draft workflow, strategy, merge, or deprecation candidates for review.
6. Draft candidates are suggestions only until they are approved.

What self-evolve does:

- Self-evolve focuses on improved routing-policy generation and testing.
- Candidate policies can be activated in canary mode so only part of traffic uses them first.
- Replay gate checks help decide whether a candidate is safe to promote.
- Promotion mode, last promotion result, and canary state explain what rollout stage the instance is in.

What the Evolution page shows:

- whether self-evolve is on
- whether learning is on
- whether learning is local-only
- learning queue counts
- canary rollout status
- learned memory
- learned procedures
- recent experience runs
- learning candidates
- strategy/canary diagnostics

How to answer user questions:

- If the user asks how self-learning works, explain the pipeline first, then the current instance status.
- If the user asks whether it is enabled or what it has learned, report the current toggles and counts first, then explain the meaning.
- Keep official product explanation separate from draft candidate content. Drafts are not live product behavior until approved.
