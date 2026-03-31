# Tasks, watchers, goals, and apps

Top-level pages:

- `Tasks`
- `Watchers`
- `Goals`
- `Apps`

How they differ:

- `Tasks`: one-off or recurring work with queue state, approvals, and retries.
- `Watchers`: background poll-until-condition workflows with timeout or trigger behavior.
- `Goals`: long-running outcomes tracked over time.
- `Apps`: built artifacts, deployed surfaces, runtime state, and public/local links.

Recommended usage:

1. Use `Tasks` when the work should run later or on a schedule.
2. Use `Watchers` when the system should keep checking until something happens.
3. Use `Goals` when the user cares about an outcome that spans multiple runs.
4. Use `Apps` when the agent built or deployed a website, dashboard, or service.

What to expect:

- Tasks can be pending, awaiting approval, running, paused, completed, failed, or cancelled.
- Watchers can be active, paused, triggered, timed out, cancelled, or failed.
- Goals are user-facing outcome trackers even though they run on top of the task system internally.
- Apps can be enabled/disabled, running/stopped, and guarded/public.

Verification:

- A scheduled job should appear in `Tasks`.
- A poll-based monitor should appear in `Watchers`.
- A live deployed surface should appear in `Apps` with at least one local or public URL when deployment succeeded.

Common issues:

- A task exists but is waiting for approval in `Mission Control`.
- A watcher is configured, but the condition never matched before timeout.
- The app files exist, but the app was not started or the runtime is degraded.
