import unittest

from bridges.gepa_optimizer import __main__ as gepa_main
from bridges.gepa_optimizer.__main__ import (
    _gepa_control_kwargs,
    _gepa_adapter,
    _gepa_lm_runtime_kwargs,
    _gepa_memory_limit_bytes,
    _gepa_optimizer_resource_kwargs,
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


class GepaControlKwargsTests(unittest.TestCase):
    def test_max_metric_calls_excludes_auto_when_supported(self) -> None:
        kwargs = _gepa_control_kwargs(
            {
                "AGENTARK_GEPA_AUTO": "light",
                "AGENTARK_GEPA_MAX_METRIC_CALLS": "24",
            },
            {"auto", "max_metric_calls"},
        )

        self.assertEqual(kwargs, {"max_metric_calls": 24})
        self.assertNotIn("auto", kwargs)

    def test_auto_is_used_when_max_metric_calls_is_absent(self) -> None:
        kwargs = _gepa_control_kwargs(
            {"AGENTARK_GEPA_AUTO": "medium"},
            {"auto", "max_metric_calls"},
        )

        self.assertEqual(kwargs, {"auto": "medium"})
        self.assertNotIn("max_metric_calls", kwargs)

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

    def test_thread_count_can_be_overridden(self) -> None:
        self.assertEqual(_gepa_thread_count({"AGENTARK_GEPA_THREADS": "3"}), 3)

    def test_lm_runtime_defaults_bound_slow_model_calls(self) -> None:
        self.assertEqual(
            _gepa_lm_runtime_kwargs({}),
            {"timeout": 60, "max_tokens": 2048, "num_retries": 0},
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
        self.assertEqual(_gepa_memory_limit_bytes({}), 1536 * 1024 * 1024)

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
                "candidate_selection_strategy": "current_best",
                "add_format_failure_as_feedback": True,
                "use_merge": False,
                "max_merge_invocations": 0,
            },
        )

    def test_optimizer_resource_limits_can_be_overridden(self) -> None:
        kwargs = _gepa_optimizer_resource_kwargs(
            {
                "AGENTARK_GEPA_REFLECTION_MINIBATCH_SIZE": "2",
                "AGENTARK_GEPA_MAX_MERGE_INVOCATIONS": "3",
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
                "candidate_selection_strategy": "current_best",
                "add_format_failure_as_feedback": True,
                "use_merge": True,
                "max_merge_invocations": 3,
            },
        )

    def test_gepa_example_defaults_keep_background_runs_small(self) -> None:
        examples = list(range(8))

        limited = _limited_gepa_examples(examples, {})
        valset = _gepa_validation_set(limited, {})

        self.assertEqual(limited, [0])
        self.assertEqual(valset, [0])

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
