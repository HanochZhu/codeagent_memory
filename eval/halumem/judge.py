"""Score HaluMem artifacts with the benchmark's own rubrics.

Recall counts only integrity score 2. Weighted recall gives score 1 half
credit, scaled by importance. Target accuracy is the precision side of
extraction F1. Interference score 0 is false-memory resistance. Update and
question labels use the benchmark's Correct / Hallucination / Omission names.
"""

from __future__ import annotations

import hashlib
import sqlite3
import threading
from concurrent.futures import ThreadPoolExecutor, as_completed

from judge_prompts import (
    EVALUATION_PROMPT_FOR_MEMORY_ACCURACY,
    EVALUATION_PROMPT_FOR_MEMORY_INTEGRITY,
    EVALUATION_PROMPT_FOR_QUESTION,
    EVALUATION_PROMPT_FOR_UPDATE_MEMORY,
)
from llm import complete_json, model_name

READER = """You answer a question using only the retrieved memories below.

1. Use the memories as the only source. Do not add facts that are not there.
2. If memories disagree, prefer the most recent one.
3. Convert relative times ("last year", "two months ago") using the memory timestamp.
4. Do not treat a character named inside a memory as the user who stored it.
5. Reply in under six words. If the memories do not contain the answer, say "unknown".

Memories:
{context}

Question: {question}

Answer:
"""

UPDATE_LABELS = {"correct": "Correct", "hallucination": "Hallucination", "omission": "Omission", "other": "Other"}
QA_LABELS = {"correct": "Correct", "hallucination": "Hallucination", "omission": "Omission"}


def _ratio(num: float, den: float) -> float | None:
    if not den:
        return None
    return num / den


def _f1(precision: float | None, recall: float | None) -> float | None:
    if precision is None or recall is None or precision + recall == 0:
        return None
    return 2 * precision * recall / (precision + recall)


def _importance(item: dict) -> float:
    try:
        return float(item.get("importance") or 0)
    except (TypeError, ValueError):
        return 0.0


def _score(value) -> int | None:
    try:
        score = int(value)
    except (TypeError, ValueError):
        return None
    if score in (0, 1, 2):
        return score
    return None


def _label(value, table: dict[str, str]) -> str | None:
    if value is None:
        return None
    return table.get(str(value).strip().lower())


class JudgeCache:
    def __init__(self, path):
        self.lock = threading.Lock()
        self.db = sqlite3.connect(path, check_same_thread=False)
        self.db.execute("create table if not exists cache (k text primary key, v text)")

    def get(self, prompt: str) -> dict | None:
        key = hashlib.sha256(f"{model_name()}\n{prompt}".encode()).hexdigest()
        with self.lock:
            row = self.db.execute("select v from cache where k = ?", (key,)).fetchone()
        if row is None:
            return None
        import json

        return json.loads(row[0])

    def put(self, prompt: str, value: dict) -> None:
        import json

        key = hashlib.sha256(f"{model_name()}\n{prompt}".encode()).hexdigest()
        with self.lock:
            self.db.execute(
                "insert or replace into cache (k, v) values (?, ?)",
                (key, json.dumps(value)),
            )
            self.db.commit()


def ask(cache: JudgeCache, prompt: str) -> dict:
    cached = cache.get(prompt)
    if cached is not None:
        return cached
    value = complete_json(prompt)
    cache.put(prompt, value)
    return value


def dialogue_text(session: dict) -> str:
    lines = []
    for turn in session.get("dialogue") or []:
        lines.append(f'[{turn.get("timestamp", "")}]{turn.get("role", "unknown")}: {turn.get("content", "")}')
        if turn.get("role") == "assistant":
            lines.append("")
    return "\n".join(lines)


def evidence_text(question: dict) -> str:
    lines = []
    for item in question.get("evidence") or []:
        if isinstance(item, dict):
            lines.append(str(item.get("memory_content") or ""))
        else:
            lines.append(str(item))
    return "\n".join(line for line in lines if line)


def judge_user(artifact: dict, cache: JudgeCache, workers: int) -> dict:
    integrity = []
    accuracy = []
    updates = []
    questions = []
    jobs = []

    for sid, session in enumerate(artifact["sessions"]):
        if session.get("is_generated_qa_session"):
            continue
        extracted = [str(item) for item in session.get("extracted_memories") or [] if str(item).strip()]
        extracted_str = "\n".join(extracted)
        dialogue = dialogue_text(session)
        golden = "\n".join(
            point["memory_content"]
            for point in session.get("memory_points") or []
            if point.get("memory_source") != "interference"
        )
        for point in session.get("memory_points") or []:
            record = dict(point)
            record["session_index"] = sid
            system_memories = point.get("memories_from_system") or []
            if str(point.get("is_update")) == "True" and system_memories:
                record["memories_from_system"] = system_memories
                updates.append(record)
                jobs.append(
                    (
                        "update",
                        record,
                        EVALUATION_PROMPT_FOR_UPDATE_MEMORY.format(
                            memories="\n".join(system_memories),
                            updated_memory=point.get("memory_content") or "",
                            original_memory="\n".join(point.get("original_memories") or []),
                        ),
                    )
                )
            else:
                integrity.append(record)
                if extracted_str.strip():
                    jobs.append(
                        (
                            "integrity",
                            record,
                            EVALUATION_PROMPT_FOR_MEMORY_INTEGRITY.format(
                                memories=extracted_str,
                                expected_memory_point=point.get("memory_content") or "",
                            ),
                        )
                    )
                else:
                    record["memory_integrity_score"] = 0
        for text in extracted:
            record = {"session_index": sid, "memory_content": text}
            accuracy.append(record)
            jobs.append(
                (
                    "accuracy",
                    record,
                    EVALUATION_PROMPT_FOR_MEMORY_ACCURACY.format(
                        dialogue=dialogue,
                        golden_memories=golden,
                        candidate_memory=text,
                    ),
                )
            )
        for question in session.get("questions") or []:
            if not question.get("system_response"):
                continue
            record = dict(question)
            record["session_index"] = sid
            questions.append(record)
            jobs.append(
                (
                    "qa",
                    record,
                    EVALUATION_PROMPT_FOR_QUESTION.format(
                        question=question.get("question") or "",
                        reference_answer=question.get("answer") or "",
                        key_memory_points=evidence_text(question),
                        response=question.get("system_response") or "",
                    ),
                )
            )

    def run(job):
        kind, record, prompt = job
        try:
            return kind, record, ask(cache, prompt)
        except Exception as exc:
            return kind, record, {"error": str(exc)[:300]}

    with ThreadPoolExecutor(max_workers=max(1, workers)) as pool:
        futures = [pool.submit(run, job) for job in jobs]
        done = 0
        total = len(futures)
        for future in as_completed(futures):
            kind, record, result = future.result()
            done += 1
            if done % 25 == 0 or done == total:
                print(f"    judged {done}/{total}", flush=True)
            if kind == "integrity":
                record["memory_integrity_score"] = _score(result.get("score"))
            elif kind == "accuracy":
                record["memory_accuracy_score"] = _score(result.get("accuracy_score"))
                included = result.get("is_included_in_golden_memories")
                record["is_included_in_golden_memories"] = str(included).lower() if included is not None else "false"
            elif kind == "update":
                record["memory_update_type"] = _label(result.get("evaluation_result"), UPDATE_LABELS)
            elif kind == "qa":
                record["result_type"] = _label(result.get("evaluation_result"), QA_LABELS)
    records = {
        "memory_integrity_records": integrity,
        "memory_accuracy_records": accuracy,
        "memory_update_records": updates,
        "question_answering_records": questions,
    }
    return {"score": aggregate(records), "records": records}


def aggregate(eval_results: dict) -> dict:
    integrity_hits = integrity_weighted = 0.0
    integrity_num = integrity_valid = 0
    integrity_weight = integrity_weight_valid = 0.0
    resisted = interference_num = interference_valid = 0
    for item in eval_results["memory_integrity_records"]:
        interference = item.get("memory_source") == "interference"
        if interference:
            interference_num += 1
        else:
            integrity_num += 1
            integrity_weight += _importance(item)
        score = item.get("memory_integrity_score")
        if score is None:
            continue
        if interference:
            interference_valid += 1
            if score == 0:
                resisted += 1
        else:
            integrity_valid += 1
            integrity_weight_valid += _importance(item)
            if score == 2:
                integrity_hits += 1
            integrity_weighted += 0.5 * score * _importance(item)

    target_hits = 0.0
    target_num = target_valid = 0
    accuracy_weighted = 0.0
    accuracy_num = accuracy_valid = 0
    for item in eval_results["memory_accuracy_records"]:
        accuracy_num += 1
        included = str(item.get("is_included_in_golden_memories")).lower() == "true"
        if included:
            target_num += 1
        score = item.get("memory_accuracy_score")
        if score is None:
            continue
        accuracy_valid += 1
        accuracy_weighted += 0.5 * score
        if included:
            target_valid += 1
            target_hits += 0.5 * score

    update_counts = {label: 0 for label in UPDATE_LABELS.values()}
    update_num = update_valid = 0
    for item in eval_results["memory_update_records"]:
        update_num += 1
        label = item.get("memory_update_type")
        if label not in update_counts:
            continue
        update_valid += 1
        update_counts[label] += 1

    qa_counts = {label: 0 for label in QA_LABELS.values()}
    qa_num = qa_valid = 0
    for item in eval_results["question_answering_records"]:
        qa_num += 1
        label = item.get("result_type")
        if label not in qa_counts:
            continue
        qa_valid += 1
        qa_counts[label] += 1

    recall = _ratio(integrity_hits, integrity_num)
    target_accuracy = _ratio(target_hits, target_num)
    return {
        "recall": recall,
        "weighted_recall": _ratio(integrity_weighted, integrity_weight),
        "target_accuracy": target_accuracy,
        "f1": _f1(target_accuracy, recall),
        "resistance": _ratio(resisted, interference_num),
        "update_correct": _ratio(update_counts["Correct"], update_num),
        "update_hallucination": _ratio(update_counts["Hallucination"], update_num),
        "update_omission": _ratio(update_counts["Omission"], update_num),
        "qa_correct": _ratio(qa_counts["Correct"], qa_num),
        "qa_hallucination": _ratio(qa_counts["Hallucination"], qa_num),
        "qa_omission": _ratio(qa_counts["Omission"], qa_num),
        "counts": {
            "integrity": integrity_num,
            "integrity_valid": integrity_valid,
            "integrity_weight": integrity_weight,
            "integrity_weight_valid": integrity_weight_valid,
            "interference": interference_num,
            "interference_valid": interference_valid,
            "resisted": resisted,
            "accuracy": accuracy_num,
            "accuracy_valid": accuracy_valid,
            "target_accuracy_num": target_num,
            "target_accuracy_valid": target_valid,
            "updates": update_num,
            "updates_valid": update_valid,
            "update_correct": update_counts["Correct"],
            "questions": qa_num,
            "questions_valid": qa_valid,
            "qa_correct": qa_counts["Correct"],
        },
    }


def combine(rows: list[dict]) -> dict:
    merged = {
        "memory_integrity_records": [],
        "memory_accuracy_records": [],
        "memory_update_records": [],
        "question_answering_records": [],
    }
    for row in rows:
        for key in merged:
            merged[key].extend(row.get(key) or [])
    return aggregate(merged)
