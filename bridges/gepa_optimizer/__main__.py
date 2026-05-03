import argparse
import inspect
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path


SUPPORTED_SURFACES = {"prompt_bundle", "specialist_prompt_bundle"}
MAX_CANDIDATE_RECORDS = 64
MAX_CANDIDATE_RECORD_BYTES = 768 * 1024
MAX_EXPORT_BYTES = 12 * 1024 * 1024


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


def _records_from_text(raw: str, run_id: str) -> list[dict]:
    raw = raw.strip()
    if not raw:
        return []
    if raw.startswith("["):
        parsed = json.loads(raw)
        if not isinstance(parsed, list):
            raise ValueError("candidate array output must be a list")
        records = parsed
    else:
        records = [json.loads(line) for line in raw.splitlines() if line.strip()]
    normalized = []
    for record in records:
        if not isinstance(record, dict):
            continue
        surface = str(record.get("surface", "")).strip()
        if surface not in SUPPORTED_SURFACES:
            continue
        candidate = record.get("candidate")
        if not isinstance(candidate, dict):
            continue
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
            continue
        normalized.append(item)
        if len(normalized) >= MAX_CANDIDATE_RECORDS:
            break
    return normalized


def _write_jsonl(path: Path, records: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        for record in records:
            handle.write(json.dumps(record, ensure_ascii=False, separators=(",", ":")))
            handle.write("\n")


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
    lm_kwargs = {}
    api_key = os.environ.get("OPENAI_API_KEY")
    if api_key:
        lm_kwargs["api_key"] = api_key
    api_base = os.environ.get("OPENAI_BASE_URL")
    if api_base:
        lm_kwargs["api_base"] = api_base
    lm = dspy.LM(model=model, **lm_kwargs)
    dspy.configure(lm=lm)
    return dspy, lm


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
        """Generate AgentArk prompt candidate JSONL from structured export evidence."""

        export_json: str = dspy.InputField(
            desc=(
                "AgentArk optimization export JSON. Use structured benchmark and trace "
                "evidence to improve behavior generally, not by matching exact user wording."
            )
        )
        candidate_jsonl: str = dspy.OutputField(
            desc=(
                "JSONL records with fields run_id, surface, source, candidate, "
                "objective_scores, feedback_summary, trace_refs, created_at. "
                "Supported surfaces are prompt_bundle and specialist_prompt_bundle."
            )
        )

    def metric(example, pred, trace=None, pred_name=None, pred_trace=None):
        try:
            records = _records_from_text(pred.candidate_jsonl, export["run_id"])
        except Exception as exc:
            return {
                "score": 0.0,
                "feedback": f"Candidate output was not parseable JSONL: {exc}",
            }
        if not records:
            return {
                "score": 0.0,
                "feedback": "No supported candidate records were produced.",
            }
        surfaces = {record["surface"] for record in records}
        quality = [_surface_candidate_quality(record) for record in records]
        quality_score = sum(item[0] for item in quality) / max(1, len(quality))
        schema_score = 0.20 + 0.20 * min(1.0, len(surfaces) / len(SUPPORTED_SURFACES))
        score = min(1.0, schema_score + 0.60 * quality_score)
        notes = [note for _, surface_notes in quality for note in surface_notes][:6]
        feedback = (
            "Candidates parsed. Preserve complete bundle schemas, explain the trace failure "
            "mode, include objective scores, and improve broad behavior instead of memorizing "
            "fixture text."
        )
        if notes:
            feedback = f"{feedback} Missing: {', '.join(notes)}."
        return {"score": score, "feedback": feedback}

    program = dspy.Predict(AgentArkCandidateSignature)
    examples = _examples_from_export(export)
    valset = examples[: max(1, min(4, len(examples)))]

    optimizer_kwargs = {
        "metric": metric,
        "auto": os.environ.get("AGENTARK_GEPA_AUTO", "light"),
        "track_stats": True,
    }
    optimizer_kwargs["reflection_lm"] = lm
    if os.environ.get("AGENTARK_GEPA_THREADS"):
        optimizer_kwargs["num_threads"] = int(os.environ["AGENTARK_GEPA_THREADS"])
    max_metric_calls = os.environ.get("AGENTARK_GEPA_MAX_METRIC_CALLS")
    if max_metric_calls and "max_metric_calls" in inspect.signature(dspy.GEPA).parameters:
        optimizer_kwargs["max_metric_calls"] = int(max_metric_calls)

    optimizer = dspy.GEPA(**optimizer_kwargs)
    optimized = optimizer.compile(program, trainset=examples, valset=valset)
    pred = optimized(export_json=json.dumps(export, ensure_ascii=False))
    return pred.candidate_jsonl


def run(args: argparse.Namespace) -> int:
    export_path = Path(args.export)
    out_path = Path(args.out)
    export = _load_export(export_path)
    raw = _run_dspy_gepa(export)
    records = _records_from_text(raw, str(export["run_id"]))
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
    args = parser.parse_args(argv)
    if args.command == "run":
        return run(args)
    raise AssertionError(args.command)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(json.dumps({"status": "failed", "error": str(exc)}), file=sys.stderr)
        raise SystemExit(1)
