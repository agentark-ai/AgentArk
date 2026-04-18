<!--
Thanks for the pull request.

AgentArk uses an issue-first contribution policy — see CONTRIBUTING.md. Pull
requests that do not reference an issue with the `approved` label are closed
automatically by the `issue-first-gate` workflow. Dependabot and maintainers
are exempt.
-->

## Summary

<!-- What does this PR change? One or two sentences. -->

## Linked approved issue

<!--
Replace `#NNN` with the actual issue number a maintainer has labeled
`approved`. Use one of: Fixes #NNN, Closes #NNN, Refs #NNN.
-->

Fixes #NNN

## Why this approach

<!--
One or two sentences on why you chose this design over the obvious alternative.
"Because the issue said so" is fine if the issue discussed alternatives.
-->

## Files touched

<!--
List the key paths you changed and what moved there, in plain English. This
helps reviewers skim the diff with context.
-->

- `path/to/file.rs` — what changed here
- `path/to/other.tsx` — what changed here

## Validation

<!-- What did you run to confirm the change works? -->

- [ ] `cargo check` passes
- [ ] `cargo test` covers the new behavior (or the change is non-code)
- [ ] Frontend `npm run build` passes (if UI changed)
- [ ] Ran the relevant integration path end-to-end (describe below)

<!-- Paste the commands you ran and their results. -->

## Security-sensitive surfaces touched

<!--
Check any that apply. If none apply, delete this section. See CONTRIBUTING.md
for what each bullet covers.
-->

- [ ] Shell or process execution
- [ ] File read/write paths
- [ ] Browser automation
- [ ] Docker runtime behavior
- [ ] Integration credentials or OAuth flows
- [ ] Public ingress, webhooks, or tunnels
- [ ] Approval bypasses or permission model changes
- [ ] Release packaging, signing, or provenance

## Docs

- [ ] Updated user-facing docs if operator behavior changed
- [ ] Updated `SECURITY.md` / `VERIFY.md` if trust surface changed
- [ ] No docs change needed

## Follow-ups or caveats

<!-- Anything you consciously left for a later PR, or known rough edges. -->
