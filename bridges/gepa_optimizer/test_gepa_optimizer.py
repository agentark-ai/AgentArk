import json
import sys
import types
import unittest

from bridges.gepa_optimizer import __main__ as gepa_main
from bridges.gepa_optimizer.__main__ import (
    _gepa_control_kwargs,
    _gepa_adapter,
    _gepa_lm_runtime_kwargs,
    _gepa_memory_limit_bytes,
    _gepa_optimizer_resource_kwargs,
    _examples_from_export,
    _gepa_thread_count,
    _gepa_validation_set,
    _fallback_candidate_records,
    _limited_gepa_examples,
    _records_from_text,
)

try:
    import dspy
except Exception:  # pragma: no cover - depends on local GEPA runtime
    dspy = None


class _FakeDspyExample:
    def __init__(self, **kwargs):
        self.__dict__.update(kwargs)

    def with_inputs(self, *_inputs):
        return self


def _examples_with_fake_dspy(export: dict, env: dict | None = None):
    previous = sys.modules.get("dspy")
    sys.modules["dspy"] = types.SimpleNamespace(Example=_FakeDspyExample)
    try:
        return _examples_from_export(export, env or {})
    finally:
        if previous is None:
            del sys.modules["dspy"]
        else:
            sys.modules["dspy"] = previous


class GepaControlKwargsTests(unittest.TestCase):
    def test_metric_call_budget_defaults_to_bounded_background_search(self) -> None:
        kwargs = _gepa_control_kwargs({}, {"auto", "max_metric_calls"})

        self.assertEqual(kwargs, {"max_metric_calls": 8})
        self.assertNotIn("auto", kwargs)

    def test_max_metric_calls_excludes_auto_when_supported(self) -> None:
        kwargs = _gepa_control_kwargs(
            {
                "AGENTARK_GEPA_AUTO": "light",
                "AGENTARK_GEPA_MAX_METRIC_CALLS": "6",
            },
            {"auto", "max_metric_calls"},
        )

        self.assertEqual(kwargs, {"max_metric_calls": 6})
        self.assertNotIn("auto", kwargs)

    def test_max_metric_calls_override_is_capped_for_background_runtime(self) -> None:
        kwargs = _gepa_control_kwargs(
            {"AGENTARK_GEPA_MAX_METRIC_CALLS": "24"},
            {"auto", "max_metric_calls"},
        )

        self.assertEqual(kwargs, {"max_metric_calls": 8})
        self.assertNotIn("auto", kwargs)

    def test_auto_is_not_used_when_default_metric_call_budget_is_supported(self) -> None:
        kwargs = _gepa_control_kwargs(
            {"AGENTARK_GEPA_AUTO": "medium"},
            {"auto", "max_metric_calls"},
        )

        self.assertEqual(kwargs, {"max_metric_calls": 8})
        self.assertNotIn("auto", kwargs)

    def test_auto_is_used_when_gepa_does_not_support_max_metric_calls(self) -> None:
        kwargs = _gepa_control_kwargs(
            {
                "AGENTARK_GEPA_AUTO": "heavy",
                "AGENTARK_GEPA_MAX_METRIC_CALLS": "24",
            },
            {"auto"},
        )

        self.assertEqual(kwargs, {"auto": "heavy"})
        self.assertNotIn("max_metric_calls", kwargs)

    def test_thread_count_defaults_to_single_threaded_execution(self) -> None:
        self.assertEqual(_gepa_thread_count({}), 1)

    def test_thread_count_can_be_overridden_within_background_cap(self) -> None:
        self.assertEqual(_gepa_thread_count({"AGENTARK_GEPA_THREADS": "2"}), 2)

    def test_thread_count_override_is_capped_for_background_cpu(self) -> None:
        self.assertEqual(_gepa_thread_count({"AGENTARK_GEPA_THREADS": "99"}), 2)

    def test_lm_runtime_defaults_bound_slow_model_calls(self) -> None:
        self.assertEqual(
            _gepa_lm_runtime_kwargs({}),
            {"timeout": 60, "max_tokens": 8192, "num_retries": 0},
        )

    def test_lm_runtime_limits_can_be_overridden(self) -> None:
        self.assertEqual(
            _gepa_lm_runtime_kwargs(
                {
                    "AGENTARK_GEPA_LM_TIMEOUT_SECONDS": "45",
                    "AGENTARK_GEPA_LM_MAX_TOKENS": "2048",
                    "AGENTARK_GEPA_LM_RETRIES": "0",
                }
            ),
            {"timeout": 45, "max_tokens": 2048, "num_retries": 0},
        )

    def test_memory_limit_defaults_to_bounded_worker_size(self) -> None:
        self.assertEqual(_gepa_memory_limit_bytes({}), 1024 * 1024 * 1024)

    def test_memory_limit_can_be_overridden_within_bounds(self) -> None:
        self.assertEqual(
            _gepa_memory_limit_bytes({"AGENTARK_GEPA_MAX_MEMORY_MB": "768"}),
            768 * 1024 * 1024,
        )

    def test_optimizer_resource_defaults_reduce_background_runtime(self) -> None:
        kwargs = _gepa_optimizer_resource_kwargs(
            {},
            {
                "reflection_minibatch_size",
                "candidate_selection_strategy",
                "add_format_failure_as_feedback",
                "use_merge",
                "max_merge_invocations",
            },
        )

        self.assertEqual(
            kwargs,
            {
                "reflection_minibatch_size": 1,
                "candidate_selection_strategy": "pareto",
                "add_format_failure_as_feedback": True,
                "use_merge": False,
                "max_merge_invocations": 0,
            },
        )

    def test_optimizer_resource_limits_can_be_overridden(self) -> None:
        kwargs = _gepa_optimizer_resource_kwargs(
            {
                "AGENTARK_GEPA_REFLECTION_MINIBATCH_SIZE": "2",
                "AGENTARK_GEPA_MAX_MERGE_INVOCATIONS": "2",
            },
            {
                "reflection_minibatch_size",
                "candidate_selection_strategy",
                "add_format_failure_as_feedback",
                "use_merge",
                "max_merge_invocations",
            },
        )

        self.assertEqual(
            kwargs,
            {
                "reflection_minibatch_size": 2,
                "candidate_selection_strategy": "pareto",
                "add_format_failure_as_feedback": True,
                "use_merge": True,
                "max_merge_invocations": 2,
            },
        )

    def test_gepa_example_defaults_scale_with_available_evidence(self) -> None:
        examples = list(range(40))

        limited = _limited_gepa_examples(examples, {})
        valset = _gepa_validation_set(limited, {})

        self.assertEqual(limited, list(range(8)))
        self.assertEqual(valset, list(range(2)))

    def test_gepa_example_defaults_keep_a_safety_cap(self) -> None:
        examples = list(range(200))

        limited = _limited_gepa_examples(examples, {})
        valset = _gepa_validation_set(limited, {})

        self.assertEqual(limited, list(range(8)))
        self.assertEqual(valset, list(range(2)))

    def test_gepa_example_override_has_a_hard_count_cap(self) -> None:
        examples = list(range(700))

        limited = _limited_gepa_examples(
            examples,
            {"AGENTARK_GEPA_MAX_EXAMPLES": "999"},
        )

        self.assertEqual(limited, list(range(64)))

    def test_examples_from_export_compacts_training_payload_without_losing_contract_context(
        self,
    ) -> None:
        large_prompt = "runtime-access-summary " * 12_000
        export = {
            "schema_version": 1,
            "run_id": "run-compact",
            "generated_at": "2026-06-12T11:35:17Z",
            "request": "Reduce Runtime Access Summary prompt weight",
            "opportunity_context": {
                "label": "Reduce Runtime Access Summary prompt weight",
                "section": "runtime_access_summary",
                "prompt_weight": {"p95_total_request_chars": 139593},
                "outcome": {"issue_rate": 0.25, "quality_issue_rate": 0.05},
                "holdout_cases": [
                    {
                        "trace_id": f"holdout-{index}",
                        "outcome": "slow_prompt",
                        "section_chars": 7000 + index,
                        "final_prompt_chars": 91000 + index,
                        "matching_samples": 3,
                    }
                    for index in range(20)
                ],
            },
            "surfaces": {
                "prompt_bundle": {
                    "version": "prompt-v1",
                    "router": {
                        "system_prompt": large_prompt,
                        "policy_block": "policy",
                        "instruction_template": "route {message}",
                    },
                    "primary_response": {
                        "system_prompt": large_prompt,
                        "policy_block": "policy",
                        "instruction_template": "answer {message}",
                    },
                },
                "prompt_fragment_bundle": {
                    "version": "fragments-v1",
                    "fragments": [
                        {
                            "id": "runtime_access_summary",
                            "surface": "prompt_bundle",
                            "body": large_prompt,
                            "enabled": True,
                        }
                    ],
                },
            },
            "benchmarks": {"giant": large_prompt},
            "recent_lineage": {"entries": [{"notes": large_prompt}] * 8},
            "candidate_contract": {
                "surfaces": ["prompt_bundle", "prompt_fragment_bundle"],
                "required_fields": ["run_id", "surface", "candidate"],
            },
            "experience_runs": [
                {
                    "trace_id": f"trace-{index}",
                    "success_state": "failed",
                    "correction_state": "needed",
                    "outcome_summary": f"Runtime prompt overweight sample {index}",
                    "failure_reason": "Large Runtime Access Summary prompt section",
                }
                for index in range(24)
            ],
        }

        examples = _examples_with_fake_dspy(export)
        payloads = [json.loads(example.export_json) for example in examples]
        encoded_total = sum(len(example.export_json.encode("utf-8")) for example in examples)
        first = payloads[0]

        self.assertEqual(len(examples), 8)
        self.assertLess(encoded_total, 256 * 1024)
        self.assertNotIn(large_prompt, examples[0].export_json)
        self.assertEqual(
            first["candidate_contract"]["surfaces"],
            ["prompt_bundle", "prompt_fragment_bundle"],
        )
        self.assertEqual(
            first["surfaces"]["prompt_bundle"]["version"],
            "prompt-v1",
        )
        self.assertIn(
            "system_prompt",
            first["surfaces"]["prompt_bundle"]["sections"]["router"]["fields"],
        )
        self.assertEqual(
            first["surfaces"]["prompt_fragment_bundle"]["fragments"][0]["id"],
            "runtime_access_summary",
        )
        self.assertEqual(len(first["opportunity_context"]["holdout_cases"]), 8)
        self.assertEqual(first["experience_runs"][0]["trace_id"], "trace-0")
        self.assertEqual(
            first["focus"]["failure_reason"],
            "Large Runtime Access Summary prompt section",
        )

    @unittest.skipIf(dspy is None, "DSPy is only installed in the GEPA runtime")
    def test_examples_from_export_preserves_all_available_runs_before_limit(self) -> None:
        export = {
            "schema_version": 1,
            "run_id": "run-1",
            "experience_runs": [
                {
                    "trace_id": f"trace-{index}",
                    "success_state": "success",
                    "correction_state": None,
                    "outcome_summary": f"run {index}",
                    "failure_reason": None,
                }
                for index in range(12)
            ],
        }

        self.assertEqual(len(_examples_from_export(export)), 8)

    @unittest.skipIf(dspy is None, "DSPy is only installed in the GEPA runtime")
    def test_examples_from_export_respects_serialized_byte_budget(self) -> None:
        export = {
            "schema_version": 1,
            "run_id": "run-1",
            "experience_runs": [
                {
                    "trace_id": f"trace-{index}",
                    "success_state": "success",
                    "correction_state": None,
                    "outcome_summary": "x" * 20_000,
                    "failure_reason": None,
                }
                for index in range(12)
            ],
        }

        examples = _examples_from_export(
            export,
            {"AGENTARK_GEPA_MAX_EXAMPLE_BYTES": "8192"},
        )

        self.assertGreaterEqual(len(examples), 1)
        self.assertLess(len(examples), 12)

    def test_records_from_text_extracts_candidate_jsonl_from_wrapped_output(self) -> None:
        raw = """
        The optimizer prepared a candidate below.

        ```jsonl
        {"run_id":"run-1","surface":"prompt_bundle","source":"test","candidate":{"version":"v2","router":{"system_prompt":"r","policy_block":"p","instruction_template":"i"},"primary_response":{"system_prompt":"r","policy_block":"p","instruction_template":"i"},"delegation_synthesis":{"system_prompt":"r","policy_block":"p","instruction_template":"i"}},"objective_scores":{"score":0.5},"feedback_summary":"schema-valid","trace_refs":["trace-1"],"created_at":"2026-01-01T00:00:00Z"}
        ```
        """

        records = _records_from_text(raw, "run-1")

        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["surface"], "prompt_bundle")
        self.assertEqual(records[0]["candidate"]["version"], "v2")

    def test_fallback_candidate_records_preserve_declared_surface_schemas(self) -> None:
        export = {
            "run_id": "run-1",
            "candidate_contract": {
                "surfaces": [
                    "prompt_bundle",
                    "specialist_prompt_bundle",
                    "arkdistill_profile",
                ]
            },
            "surfaces": {
                "prompt_bundle": {
                    "version": "prompt-v1",
                    "router": {
                        "system_prompt": "router",
                        "policy_block": "policy",
                        "instruction_template": "task {message}",
                    },
                    "primary_response": {
                        "system_prompt": "primary",
                        "policy_block": "policy",
                        "instruction_template": "final",
                    },
                    "delegation_synthesis": {
                        "system_prompt": "synthesis",
                        "policy_block": "policy",
                        "instruction_template": "{original_task} {results_text}",
                    },
                },
                "specialist_prompt_bundle": {
                    "version": "specialist-v1",
                    "researcher": {"system_prompt": "research"},
                },
            },
            "experience_runs": [{"trace_id": "trace-1"}],
        }

        records = _fallback_candidate_records(export, "format_unavailable")

        self.assertEqual(
            [record["surface"] for record in records],
            ["prompt_bundle", "specialist_prompt_bundle"],
        )
        self.assertEqual(records[0]["run_id"], "run-1")
        self.assertEqual(records[0]["candidate"]["router"]["system_prompt"], "router")
        self.assertNotEqual(records[0]["candidate"]["version"], "prompt-v1")
        self.assertIn("trace-1", records[0]["trace_refs"])

    @unittest.skipIf(dspy is None, "DSPy is only installed in the GEPA runtime")
    def test_gepa_adapter_extracts_fields_from_model_prose(self) -> None:
        lm = dspy.LM(model="openai/test-model", api_key="test")

        self.assertIsInstance(_gepa_adapter(dspy, lm), dspy.TwoStepAdapter)

    @unittest.skipIf(dspy is None, "DSPy is only installed in the GEPA runtime")
    def test_score_with_feedback_uses_dspy_prediction_contract(self) -> None:
        result = gepa_main._score_with_feedback(dspy, 0.5, "schema feedback")

        self.assertEqual(result["score"], 0.5)
        self.assertEqual(result["feedback"], "schema feedback")
        self.assertTrue(hasattr(result, "score"))


if __name__ == "__main__":
    unittest.main()
