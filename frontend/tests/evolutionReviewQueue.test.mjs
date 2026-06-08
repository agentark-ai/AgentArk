import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const evolutionPageSource = readFileSync(
  path.join(frontendRoot, "src", "components", "pages", "EvolutionPage.tsx"),
  "utf8",
);

test("prompt optimization proposal copy uses approval and next-stage language", () => {
  assert.doesNotMatch(evolutionPageSource, /Save(?:d)? for follow-up/i);
  assert.match(evolutionPageSource, /"Approved"/);
  assert.match(evolutionPageSource, />\s*Approve next stage\s*</);
});

test("prompt optimization rows open the review dialog directly", () => {
  assert.match(evolutionPageSource, /component="button"/);
  assert.match(evolutionPageSource, /setTechnicalDialogProposalId\(proposalId\)/);
  assert.doesNotMatch(evolutionPageSource, /const primaryAction\s*=/);
});

test("prompt optimization rows show priority and estimated savings", () => {
  assert.match(evolutionPageSource, /Ranked by estimated savings/);
  assert.match(evolutionPageSource, /Top opportunity/);
  assert.match(evolutionPageSource, /Estimated p95 savings/);
  assert.match(evolutionPageSource, /estimated_saved_tokens_p95/);
  assert.match(evolutionPageSource, /Evidence confidence/);
  assert.match(evolutionPageSource, /confidence_sample_target/);
  assert.match(evolutionPageSource, /sample_confidence_score/);
});

test("prompt optimization proposal modal is visual first with collapsed evidence", () => {
  assert.doesNotMatch(evolutionPageSource, /promptFootprintChartOption/);
  assert.match(evolutionPageSource, /promptHoldoutFootprintChartOption/);
  assert.match(evolutionPageSource, /Target section/);
  assert.match(evolutionPageSource, /Rest of prompt/);
  assert.match(evolutionPageSource, /Validation samples/);
  assert.match(evolutionPageSource, /Evidence details/);
  assert.doesNotMatch(evolutionPageSource, /promptOptimizationReviewEvidence/);
});

test("prompt optimization validation chart uses readable case labels", () => {
  assert.doesNotMatch(evolutionPageSource, /`S\$\{idx \+ 1\}`/);
  assert.match(evolutionPageSource, /promptHoldoutFootprintRows/);
  assert.match(evolutionPageSource, /Target section chars/);
  assert.match(evolutionPageSource, /Rest of prompt chars/);
});

test("dismissed prompt optimization proposals can be approved from past decisions", () => {
  assert.match(evolutionPageSource, /const isPromptProposalDismissed = reviewStatus === "rejected"/);
  assert.match(evolutionPageSource, /const canApprove =\s*!!proposalId && !isPromptProposalApproved/);
  assert.match(
    evolutionPageSource,
    /const canDismiss =\s*!!proposalId && !isPromptProposalApproved && !isPromptProposalDismissed/,
  );
  assert.match(evolutionPageSource, /canDismiss \? \(/);
});

test("dismissing a prompt optimization proposal asks for confirmation", () => {
  assert.match(evolutionPageSource, /Dismiss this suggestion\?/);
  assert.match(evolutionPageSource, /approve it later/);
});

test("evolve separates arkdistill context savings from prompt optimization decisions", () => {
  assert.match(evolutionPageSource, /arkdistill_context_summary/);
  assert.match(evolutionPageSource, /ArkDistill context savings/);
  assert.match(evolutionPageSource, /Prompt-section proposals below/);
});

test("blocked prompt optimization proposals do not look like a first background run", () => {
  assert.doesNotMatch(
    evolutionPageSource,
    /const canRunPromptBackgroundTest =[\s\S]{0,260}lifecycleStatus === "blocked"/,
  );
  assert.match(evolutionPageSource, /canRetryPromptBackgroundTest/);
  assert.match(evolutionPageSource, /"Retry background test"/);
  assert.match(evolutionPageSource, /Block reason/);
});

test("prompt lifecycle reasons are highlighted and novice readable", () => {
  assert.match(evolutionPageSource, /function formatPromptLifecycleReason/);
  assert.match(
    evolutionPageSource,
    /formatPromptLifecycleReason\(lifecycleReason\)/,
  );
  assert.match(evolutionPageSource, /daily spending limit for background optimization/);
  assert.match(evolutionPageSource, /replace\(\/\[_\\s-\]\+\/g, " "\)/);
  assert.match(evolutionPageSource, />\s*Reason\s*</);
  assert.match(evolutionPageSource, /borderLeft: `3px solid \$\{reasonAccent\}`/);
});

test("rejected GEPA prompt candidates are retryable and not labeled ready", () => {
  assert.match(evolutionPageSource, /case "candidate_rejected":\s*return "Not promoted"/);
  assert.match(
    evolutionPageSource,
    /canRetryPromptBackgroundTest =[\s\S]{0,360}lifecycleStatus === "candidate_rejected"/,
  );
  assert.match(evolutionPageSource, /Candidate was not promoted/);
});

test("rejected GEPA prompt candidates are past decisions by default", () => {
  assert.match(evolutionPageSource, /function promptProposalIsPastDecision/);
  assert.match(
    evolutionPageSource,
    /promptProposalIsPastDecision[\s\S]{0,420}lifecycleStatus === "candidate_rejected"/,
  );
  assert.match(
    evolutionPageSource,
    /activePromptOptimizationOpportunities =[\s\S]{0,260}promptProposalIsPastDecision\(row\)/,
  );
});

test("prompt canary deployment retains live deploy rollback and monitoring controls", () => {
  assert.match(
    evolutionPageSource,
    /canManagePromptCanary =[\s\S]{0,260}lifecycleStatus === "testing"[\s\S]{0,260}lifecycleStatus === "deployed"[\s\S]{0,260}lifecycleStatus === "rollback_suggested"/,
  );
  assert.match(evolutionPageSource, /action: "disable_prompt_canary"/);
  assert.match(evolutionPageSource, />\s*Stop test\s*</);
  assert.match(evolutionPageSource, /action: "promote_prompt_canary_candidate"/);
  assert.match(evolutionPageSource, />\s*Deploy to AgentArk\s*</);
  assert.match(evolutionPageSource, /action: "rollback_prompt_baseline"/);
  assert.match(evolutionPageSource, />\s*Roll back\s*</);
  assert.match(evolutionPageSource, /promptMonitoringChartOption\(lifecycle\)/);
  assert.match(evolutionPageSource, /monitoringRegressions\.length > 0/);
  assert.match(evolutionPageSource, /monitoringSummary\.length > 0/);
});
