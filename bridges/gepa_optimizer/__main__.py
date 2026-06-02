import argparse
import inspect
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Mapping


SUPPORTED_SURFACES = {
    "prompt_bundle",
    "specialist_prompt_bundle",
    "prompt_fragment_bundle",
    "arkdistill_profile",
    "router_learning",
}
MAX_CANDIDATE_RECORDS = 64
MAX_CANDIDATE_RECORD_BYTES = 768 * 1024
MAX_EXPORT_BYTES = 12 * 1024 * 1024
DEFAULT_GEPA_LM_TIMEOUT_SECONDS = 60
DEFAULT_GEPA_LM_MAX_TOKENS = 2048
DEFAULT_GEPA_LM_RETRIES = 0
DEFAULT_GEPA_MAX_EXAMPLES = 1
DEFAULT_GEPA_VALSET_SIZE = 1
DEFAULT_GEPA_MAX_MEMORY_MB = 1536
DEFAULT_GEPA_REFLECTION_MINIBATCH_SIZE = 1
DEFAULT_GEPA_MAX_MERGE_INVOCATIONS = 0
DEFAULT_GEPA_CANDIDATE_SELECTION_STRATEGY = "current_best"


def _bounded_int(
    env: Mapping[str, str],
    key: str,
    default: int,
    minimum: int,
    maximum: int,
) -> int:
    raw = str(env.get(key) or "").strip()
    if not raw:
        return default
    return min(maximum, max(minimum, int(raw)))


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def _load_export(path: Path) -> dict:
    if path.stat().st_size > MAX_EXPORT_BYTES:
        raise ValueError(f"export file exceeds {MAX_EXPORT_BYTES} bytes")
    with path.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise ValueError("export root must be a JSON object")
    if data.get("schema_version") != 1:
        raise ValueError("unsupported export schema_version")
    return data


def _normalize_candidate_record(record: dict, run_id: str) -> dict | None:
    surface = str(record.get("surface", "")).strip()
    if surface not in SUPPORTED_SURFACES:
        return None
    candidate = record.get("candidate")
    if not isinstance(candidate, dict):
        return None
    item = {
        "run_id": str(record.get("run_id") or run_id),
        "surface": surface,
        "source": str(record.get("source") or "dspy-gepa"),
        "candidate": candidate,
        "objective_scores": record.get("objective_scores") or {},
        "feedback_summary": str(record.get("feedback_summary") or ""),
        "trace_refs": [
            str(item)
            for item in record.get("trace_refs", [])
            if isinstance(item, (str, int, float))
        ],
        "created_at": str(record.get("created_at") or _utc_now()),
    }
    encoded_len = len(json.dumps(item, ensure_ascii=False).encode("utf-8"))
    if encoded_len > MAX_CANDIDATE_RECORD_BYTES:
        return None
    return item


def _candidate_records_from_value(value, run_id: str, depth: int = 0) -> list[dict]:
    if depth > 6:
        return []
    if isinstance(value, dict):
        normalized = _normalize_candidate_record(value, run_id)
        if normalized is not None:
            return [normalized]
        records: list[dict] = []
        candidate_jsonl = value.get("candidate_jsonl")
        if isinstance(candidate_jsonl, str):
            records.extend(_records_from_text(candidate_jsonl, run_id))
        for nested in value.values():
            if isinstance(nested, (dict, list)):
                records.extend(_candidate_records_from_value(nested, run_id, depth + 1))
        return records
    if isinstance(value, list):
        records = []
        for item in value:
            records.extend(_candidate_records_from_value(item, run_id, depth + 1))
            if len(records) >= MAX_CANDIDATE_RECORDS:
                break
        return records
    return []


def _iter_json_values(raw: str):
    decoder = json.JSONDecoder()
    idx = 0
    while idx < len(raw):
        object_start = raw.find("{", idx)
        array_start = raw.find("[", idx)
        starts = [pos for pos in (object_start, array_start) if pos >= 0]
        if not starts:
            return
        start = min(starts)
        try:
            value, end = decoder.raw_decode(raw[start:])
        except json.JSONDecodeError:
            idx = start + 1
            continue
        yield value
        idx = start + max(end, 1)


def _append_unique_records(target: list[dict], records: list[dict]) -> None:
    seen = {
        json.dumps(
            {
                "run_id": record.get("run_id"),
                "surface": record.get("surface"),
                "source": record.get("source"),
                "candidate": record.get("candidate"),
            },
            ensure_ascii=False,
            sort_keys=True,
        )
        for record in target
    }
    for record in records:
        key = json.dumps(
            {
                "run_id": record.get("run_id"),
                "surface": record.get("surface"),
                "source": record.get("source"),
                "candidate": record.get("candidate"),
            },
            ensure_ascii=False,
            sort_keys=True,
        )
        if key in seen:
            continue
        target.append(record)
        seen.add(key)
        if len(target) >= MAX_CANDIDATE_RECORDS:
            break


def _records_from_text(raw: str, run_id: str) -> list[dict]:
    raw = raw.strip()
    if not raw:
        return []
    normalized: list[dict] = []
    for line in raw.splitlines():
        line = line.strip()
        if not line or line[0] not in "{[":
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        _append_unique_records(
            normalized, _candidate_records_from_value(value, run_id)
        )
        if len(normalized) >= MAX_CANDIDATE_RECORDS:
            return normalized
    for value in _iter_json_values(raw):
        _append_unique_records(normalized, _candidate_records_from_value(value, run_id))
        if len(normalized) >= MAX_CANDIDATE_RECORDS:
            break
    return normalized


def _trace_refs_from_export(export: dict) -> list[str]:
    refs: list[str] = []
    for run in export.get("experience_runs", []):
        if not isinstance(run, dict):
            continue
        for key in ("trace_id", "id"):
            value = run.get(key)
            if isinstance(value, (str, int, float)):
                refs.append(str(value))
                break
    opportunity = export.get("opportunity_context")
    if isinstance(opportunity, dict):
        for case in opportunity.get("holdout_cases", []):
            if not isinstance(case, dict):
                continue
            value = case.get("trace_id") or case.get("id")
            if isinstance(value, (str, int, float)):
                refs.append(str(value))
    deduped: list[str] = []
    seen: set[str] = set()
    for ref in refs:
        if ref and ref not in seen:
            deduped.append(ref)
            seen.add(ref)
        if len(deduped) >= 12:
            break
    return deduped


def _fallback_candidate_records(export: dict, reason: str) -> list[dict]:
    run_id = str(export.get("run_id") or "gepa-fallback")
    surfaces = export.get("surfaces") if isinstance(export.get("surfaces"), dict) else {}
    contract = (
        export.get("candidate_contract")
        if isinstance(export.get("candidate_contract"), dict)
        else {}
    )
    declared_surfaces = [
        str(surface).strip()
        for surface in contract.get("surfaces", [])
        if isinstance(surface, str) and str(surface).strip()
    ]
    ordered_surfaces = declared_surfaces + [
        surface for surface in surfaces.keys() if surface not in declared_surfaces
    ]
    trace_refs = _trace_refs_from_export(export)
    records = []
    for surface in ordered_surfaces:
        if surface not in SUPPORTED_SURFACES:
            continue
        candidate = surfaces.get(surface)
        if not isinstance(candidate, dict):
            continue
        candidate = json.loads(json.dumps(candidate, ensure_ascii=False))
        version = str(candidate.get("version") or surface).strip() or surface
        candidate["version"] = f"{version}+gepa-schema-fallback"
        records.append(
            {
                "run_id": run_id,
                "surface": surface,
                "source": "dspy-gepa-schema-fallback",
                "candidate": candidate,
                "objective_scores": {
                    "schema_valid": 1.0,
                    "behavior_change": 0.0,
                    "fallback": 1.0,
                },
                "feedback_summary": (
                    "Optimizer output did not yield parseable candidate records; "
                    "emitted a schema-preserving baseline candidate for downstream "
                    f"benchmark and promotion gates. Reason: {reason}"
                ),
                "trace_refs": trace_refs,
                "created_at": _utc_now(),
            }
        )
        if len(records) >= MAX_CANDIDATE_RECORDS:
            break
    return records


def _write_jsonl(path: Path, records: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        for record in records:
            handle.write(json.dumps(record, ensure_ascii=False, separators=(",", ":")))
            handle.write("\n")


def _gepa_control_kwargs(env: Mapping[str, str], gepa_parameters: set[str]) -> dict:
    max_metric_calls = str(env.get("AGENTARK_GEPA_MAX_METRIC_CALLS") or "").strip()
    if max_metric_calls and "max_metric_calls" in gepa_parameters:
        return {"max_metric_calls": int(max_metric_calls)}

    auto = str(env.get("AGENTARK_GEPA_AUTO") or "light").strip() or "light"
    return {"auto": auto}


def _gepa_thread_count(env: Mapping[str, str]) -> int:
    raw = str(env.get("AGENTARK_GEPA_THREADS") or "").strip()
    if not raw:
        return 1
    return max(1, int(raw))


def _gepa_lm_runtime_kwargs(env: Mapping[str, str]) -> dict:
    return {
        "timeout": _bounded_int(
            env,
            "AGENTARK_GEPA_LM_TIMEOUT_SECONDS",
            DEFAULT_GEPA_LM_TIMEOUT_SECONDS,
            15,
            600,
        ),
        "max_tokens": _bounded_int(
            env,
            "AGENTARK_GEPA_LM_MAX_TOKENS",
            DEFAULT_GEPA_LM_MAX_TOKENS,
            512,
            32768,
        ),
        "num_retries": _bounded_int(
            env,
            "AGENTARK_GEPA_LM_RETRIES",
            DEFAULT_GEPA_LM_RETRIES,
            0,
            5,
        ),
    }


def _gepa_memory_limit_bytes(env: Mapping[str, str]) -> int:
    memory_mb = _bounded_int(
        env,
        "AGENTARK_GEPA_MAX_MEMORY_MB",
        DEFAULT_GEPA_MAX_MEMORY_MB,
        512,
        8192,
    )
    return memory_mb * 1024 * 1024


def _apply_process_resource_limits(env: Mapping[str, str]) -> None:
    try:
        import resource  # type: ignore
    except Exception:
        return
    limit_bytes = _gepa_memory_limit_bytes(env)
    try:
        soft, hard = resource.getrlimit(resource.RLIMIT_AS)
        if hard not in (-1, resource.RLIM_INFINITY):
            limit_bytes = min(limit_bytes, int(hard))
        resource.setrlimit(resource.RLIMIT_AS, (limit_bytes, hard))
    except Exception:
        return


def _gepa_optimizer_resource_kwargs(env: Mapping[str, str], gepa_parameters: set[str]) -> dict:
    kwargs = {}
    if "reflection_minibatch_size" in gepa_parameters:
        kwargs["reflection_minibatch_size"] = _bounded_int(
            env,
            "AGENTARK_GEPA_REFLECTION_MINIBATCH_SIZE",
            DEFAULT_GEPA_REFLECTION_MINIBATCH_SIZE,
            1,
            8,
        )
    if "candidate_selection_strategy" in gepa_parameters:
        kwargs["candidate_selection_strategy"] = str(
            env.get("AGENTARK_GEPA_CANDIDATE_SELECTION_STRATEGY")
            or DEFAULT_GEPA_CANDIDATE_SELECTION_STRATEGY
        ).strip() or DEFAULT_GEPA_CANDIDATE_SELECTION_STRATEGY
    if "add_format_failure_as_feedback" in gepa_parameters:
        kwargs["add_format_failure_as_feedback"] = True
    if "max_merge_invocations" in gepa_parameters:
        max_merge_invocations = _bounded_int(
            env,
            "AGENTARK_GEPA_MAX_MERGE_INVOCATIONS",
            DEFAULT_GEPA_MAX_MERGE_INVOCATIONS,
            0,
            10,
        )
        kwargs["max_merge_invocations"] = max_merge_invocations
        if "use_merge" in gepa_parameters:
            kwargs["use_merge"] = max_merge_invocations > 0
    elif "use_merge" in gepa_parameters:
        kwargs["use_merge"] = False
    return kwargs


def _score_with_feedback(dspy, score: float, feedback: str):
    return dspy.Prediction(score=float(score), feedback=str(feedback))


def _gepa_adapter(dspy, lm):
    return dspy.TwoStepAdapter(extraction_model=lm)


def _gepa_optimizer_kwargs(dspy, lm, metric) -> dict:
    gepa_parameters = set(inspect.signature(dspy.GEPA).parameters)
    optimizer_kwargs = {
        "metric": metric,
        "track_stats": False,
        **_gepa_control_kwargs(os.environ, gepa_parameters),
        **_gepa_optimizer_resource_kwargs(os.environ, gepa_parameters),
    }
    optimizer_kwargs["reflection_lm"] = lm
    if "num_threads" in gepa_parameters:
        optimizer_kwargs["num_threads"] = _gepa_thread_count(os.environ)
    return optimizer_kwargs


def _configure_dspy():
    try:
        import dspy  # type: ignore
    except Exception as exc:  # pragma: no cover - depends on dev env
        raise RuntimeError(
            "DSPy is not installed. Install bridges/gepa_optimizer/requirements.txt "
            "before running offline GEPA optimization."
        ) from exc

    model = os.environ.get("AGENTARK_GEPA_MODEL") or os.environ.get("OPENAI_MODEL")
    if not model:
        raise RuntimeError(
            "AgentArk did not provide a GEPA model. Configure the active AgentArk model first."
        )
    lm_kwargs = _gepa_lm_runtime_kwargs(os.environ)
    api_key = os.environ.get("OPENAI_API_KEY")
    if api_key:
        lm_kwargs["api_key"] = api_key
    api_base = os.environ.get("OPENAI_BASE_URL")
    if api_base:
        lm_kwargs["api_base"] = api_base
    lm = dspy.LM(model=model, **lm_kwargs)
    dspy.configure(lm=lm, adapter=_gepa_adapter(dspy, lm))
    return dspy, lm


def _limited_gepa_examples(examples, env: Mapping[str, str]):
    max_examples = _bounded_int(
        env,
        "AGENTARK_GEPA_MAX_EXAMPLES",
        DEFAULT_GEPA_MAX_EXAMPLES,
        1,
        32,
    )
    return list(examples)[:max_examples]


def _gepa_validation_set(examples, env: Mapping[str, str]):
    items = list(examples)
    valset_size = _bounded_int(
        env,
        "AGENTARK_GEPA_VALSET_SIZE",
        DEFAULT_GEPA_VALSET_SIZE,
        1,
        32,
    )
    return items[: max(1, min(valset_size, len(items)))]


def _surface_candidate_quality(record: dict) -> tuple[float, list[str]]:
    surface = record.get("surface")
    candidate = record.get("candidate") if isinstance(record.get("candidate"), dict) else {}
    notes: list[str] = []
    score = 0.0
    if surface == "prompt_bundle":
        expected = ("version", "router", "primary_response", "delegation_synthesis")
        present = [key for key in expected if isinstance(candidate.get(key), (str, dict))]
        score += 0.45 * (len(present) / len(expected))
        for section in ("router", "primary_response", "delegation_synthesis"):
            value = candidate.get(section)
            if isinstance(value, dict) and all(
                isinstance(value.get(field), str) and value.get(field, "").strip()
                for field in ("system_prompt", "policy_block", "instruction_template")
            ):
                score += 0.10
            else:
                notes.append(f"{section} is incomplete")
    elif surface == "specialist_prompt_bundle":
        expected = ("researcher", "coder", "analyst", "writer", "validator", "planner")
        present = [key for key in expected if isinstance(candidate.get(key), dict)]
        score += 0.45 * (len(present) / len(expected))
        for role in expected:
            value = candidate.get(role)
            if isinstance(value, dict) and isinstance(value.get("system_prompt"), str):
                score += 0.05
            else:
                notes.append(f"{role} is incomplete")
    elif surface == "prompt_fragment_bundle":
        fragments = candidate.get("fragments")
        if isinstance(fragments, list) and fragments:
            complete = 0
            enabled = 0
            for fragment in fragments[:64]:
                if not isinstance(fragment, dict):
                    continue
                if fragment.get("enabled", True):
                    enabled += 1
                if (
                    isinstance(fragment.get("id"), str)
                    and fragment.get("id", "").strip()
                    and isinstance(fragment.get("surface"), str)
                    and fragment.get("surface", "").strip()
                    and isinstance(fragment.get("body"), str)
                    and fragment.get("body", "").strip()
                ):
                    complete += 1
            score += 0.50 * min(1.0, complete / max(1, min(len(fragments), 12)))
            score += 0.10 if enabled else 0.0
        else:
            notes.append("fragments is empty or missing")
    elif surface == "arkdistill_profile":
        if candidate.get("enabled", True) is False:
            notes.append("profile disables ArkDistill")
        limits = candidate.get("generic_limits")
        rules = candidate.get("rules")
        if isinstance(limits, dict):
            score += 0.25
        else:
            notes.append("generic_limits is missing")
        if isinstance(rules, list) and rules:
            complete = 0
            for rule in rules[:24]:
                if not isinstance(rule, dict):
                    continue
                if (
                    isinstance(rule.get("field_names"), list)
                    or isinstance(rule.get("field_paths"), list)
                    or isinstance(rule.get("omit_field_names"), list)
                ):
                    complete += 1
            score += 0.35 * min(1.0, complete / max(1, min(len(rules), 8)))
        else:
            notes.append("rules is empty or missing")
        required = candidate.get("required_fields")
        if isinstance(required, list) and required:
            score += 0.15
        else:
            notes.append("required_fields is empty or missing")
    elif surface == "router_learning":
        if candidate:
            score += 0.60
        else:
            notes.append("router_learning candidate is empty")
    if isinstance(candidate.get("version"), str) and candidate["version"].strip():
        score += 0.15
    if isinstance(record.get("feedback_summary"), str) and record["feedback_summary"].strip():
        score += 0.10
    if isinstance(record.get("trace_refs"), list) and record["trace_refs"]:
        score += 0.10
    if isinstance(record.get("objective_scores"), dict) and record["objective_scores"]:
        score += 0.10
    return min(1.0, score), notes


def _examples_from_export(export: dict):
    try:
        import dspy  # type: ignore
    except Exception as exc:  # pragma: no cover - already checked by caller
        raise RuntimeError("DSPy is not available while building GEPA examples") from exc

    base = {
        "schema_version": export.get("schema_version"),
        "run_id": export.get("run_id"),
        "generated_at": export.get("generated_at"),
        "request": export.get("request"),
        "opportunity_context": export.get("opportunity_context"),
        "surfaces": export.get("surfaces", {}),
        "benchmarks": export.get("benchmarks", {}),
        "recent_lineage": export.get("recent_lineage", {}),
        "candidate_contract": export.get("candidate_contract", {}),
    }
    runs = export.get("experience_runs")
    examples = []
    if isinstance(runs, list) and runs:
        for run in runs[:8]:
            if not isinstance(run, dict):
                continue
            payload = dict(base)
            payload["experience_runs"] = [run]
            payload["focus"] = {
                "success_state": run.get("success_state"),
                "correction_state": run.get("correction_state"),
                "outcome_summary": run.get("outcome_summary"),
                "failure_reason": run.get("failure_reason"),
            }
            examples.append(
                dspy.Example(export_json=json.dumps(payload, ensure_ascii=False)).with_inputs(
                    "export_json"
                )
            )
    if not examples:
        examples.append(
            dspy.Example(export_json=json.dumps(export, ensure_ascii=False)).with_inputs(
                "export_json"
            )
        )
    return examples


def _run_dspy_gepa(export: dict) -> str:
    dspy, lm = _configure_dspy()

    class AgentArkCandidateSignature(dspy.Signature):
        """Generate AgentArk candidate JSONL from structured export evidence.

        The candidate_jsonl output must contain only newline-delimited JSON
        records that satisfy export_json.candidate_contract. Each record must
        include run_id, surface, source, candidate, objective_scores,
        feedback_summary, trace_refs, and created_at. Candidate objects must be
        complete profiles for their surface; preserve unchanged fields from
        export_json.surfaces and change only what the evidence supports.
        """

        export_json: str = dspy.InputField(
            desc=(
                "AgentArk optimization export JSON. Use structured benchmark and trace "
                "evidence, opportunity context, and holdout cases to improve behavior "
                "generally, not by matching exact user wording."
            )
        )
        candidate_jsonl: str = dspy.OutputField(
            desc=(
                "Only JSONL, with one JSON object per line. Each line has "
                "run_id, surface, source, candidate, objective_scores, "
                "feedback_summary, trace_refs, created_at. Supported surfaces "
                "are prompt_bundle, specialist_prompt_bundle, "
                "prompt_fragment_bundle, arkdistill_profile, and "
                "router_learning. Do not include prose, markdown, or analysis "
                "outside the JSONL field."
            )
        )

    def metric(example, pred, trace=None, pred_name=None, pred_trace=None):
        try:
            records = _records_from_text(pred.candidate_jsonl, export["run_id"])
        except Exception as exc:
            return _score_with_feedback(
                dspy, 0.0, f"Candidate output was not parseable JSONL: {exc}"
            )
        if not records:
            return _score_with_feedback(
                dspy, 0.0, "No supported candidate records were produced."
            )
        surfaces = {record["surface"] for record in records}
        quality = [_surface_candidate_quality(record) for record in records]
        quality_score = sum(item[0] for item in quality) / max(1, len(quality))
        schema_score = 0.20 + 0.20 * min(1.0, len(surfaces) / len(SUPPORTED_SURFACES))
        score = min(1.0, schema_score + 0.60 * quality_score)
        notes = [note for _, surface_notes in quality for note in surface_notes][:6]
        feedback = (
            "Candidates parsed. Preserve complete profile schemas, explain the trace failure "
            "mode, include objective scores, and improve broad behavior instead of memorizing "
            "fixture text."
        )
        if notes:
            feedback = f"{feedback} Missing: {', '.join(notes)}."
        return _score_with_feedback(dspy, score, feedback)

    program = dspy.Predict(AgentArkCandidateSignature)
    examples = _limited_gepa_examples(_examples_from_export(export), os.environ)
    valset = _gepa_validation_set(examples, os.environ)

    optimizer = dspy.GEPA(**_gepa_optimizer_kwargs(dspy, lm, metric))
    optimized = optimizer.compile(program, trainset=examples, valset=valset)
    pred = optimized(export_json=json.dumps(export, ensure_ascii=False))
    return pred.candidate_jsonl


def validate(args: argparse.Namespace) -> int:
    _apply_process_resource_limits(os.environ)
    dspy, lm = _configure_dspy()

    def metric(gold, pred, trace=None, pred_name=None, pred_trace=None):
        return _score_with_feedback(dspy, 1.0, "preflight")

    optimizer_kwargs = _gepa_optimizer_kwargs(dspy, lm, metric)
    dspy.GEPA(**optimizer_kwargs)
    controls = [
        key
        for key in ("max_metric_calls", "max_full_evals", "auto")
        if key in optimizer_kwargs
    ]
    print(json.dumps({"status": "ok", "controls": controls}))
    return 0


def run(args: argparse.Namespace) -> int:
    _apply_process_resource_limits(os.environ)
    export_path = Path(args.export)
    out_path = Path(args.out)
    export = _load_export(export_path)
    try:
        raw = _run_dspy_gepa(export)
        records = _records_from_text(raw, str(export["run_id"]))
    except Exception as exc:
        records = _fallback_candidate_records(export, type(exc).__name__)
        if records:
            print(
                json.dumps(
                    {
                        "status": "warning",
                        "warning": "optimizer_output_unavailable",
                        "fallback_records": len(records),
                        "error_type": type(exc).__name__,
                    }
                ),
                file=sys.stderr,
            )
        else:
            raise
    if not records:
        records = _fallback_candidate_records(export, "no_parseable_candidate_records")
    if not records:
        raise RuntimeError("GEPA completed but produced no valid AgentArk candidates")
    _write_jsonl(out_path, records)
    print(json.dumps({"status": "completed", "records": len(records), "out": str(out_path)}))
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m bridges.gepa_optimizer")
    subparsers = parser.add_subparsers(dest="command", required=True)
    run_parser = subparsers.add_parser("run")
    run_parser.add_argument("--export", required=True)
    run_parser.add_argument("--out", required=True)
    subparsers.add_parser("validate")
    args = parser.parse_args(argv)
    if args.command == "run":
        return run(args)
    if args.command == "validate":
        return validate(args)
    raise AssertionError(args.command)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(json.dumps({"status": "failed", "error": str(exc)}), file=sys.stderr)
        raise SystemExit(1)
