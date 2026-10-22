"""Lexical scores for HaluMem memory points stored by cam.

A gold memory is covered when one stored memory holds at least `threshold`
of its content words. Official HaluMem uses an LLM judge; these rates are
not that leaderboard. An update counts when the memory that covers the new
text supersedes a memory that covers one of the replaced texts.
"""

from __future__ import annotations

import re

THRESHOLD = 0.55


def content_words(text: str) -> list[str]:
    return [word for word in re.findall(r"[a-z0-9']+", text.casefold()) if len(word) > 2]


def coverage(needle: str, hay: str) -> float:
    words = content_words(needle)
    if not words:
        return 0.0
    present = set(content_words(hay))
    return sum(word in present for word in words) / len(words)


def blob(memory: dict) -> str:
    return f"{memory.get('summary') or ''}\n{memory.get('body') or ''}"


def best_match(text: str, memories: list[dict], threshold: float = THRESHOLD) -> dict | None:
    found = None
    best = 0.0
    for memory in memories:
        score = coverage(text, blob(memory))
        if score > best:
            found, best = memory, score
    if found is not None and best >= threshold:
        return found
    return None


def is_update(point: dict) -> bool:
    return str(point.get("is_update")).lower() == "true"


def is_interference(point: dict) -> bool:
    return point.get("memory_source") == "interference"


def score_bank(memories: list[dict], points: list[dict], threshold: float = THRESHOLD) -> dict:
    target = [point for point in points if not is_interference(point)]
    noise = [point for point in points if is_interference(point)]
    covered = sum(1 for point in target if best_match(point["memory_content"], memories, threshold))
    resisted = sum(1 for point in noise if best_match(point["memory_content"], memories, threshold) is None)
    precise = 0
    for memory in memories:
        if any(coverage(point["memory_content"], blob(memory)) >= threshold for point in target):
            precise += 1
    recall = covered / len(target) if target else 0.0
    precision = precise / len(memories) if memories else 0.0
    f1 = 0.0 if precision + recall == 0 else 2 * precision * recall / (precision + recall)

    update_hits = update_total = 0
    for point in points:
        originals = [item for item in (point.get("original_memories") or []) if item]
        if not is_update(point) or not originals:
            continue
        update_total += 1
        new = best_match(point["memory_content"], memories, threshold)
        old_ids = {
            match["id"]
            for original in originals
            if (match := best_match(original, memories, threshold))
        }
        if new and new.get("supersedes_id") in old_ids:
            update_hits += 1

    return {
        "memories": len(memories),
        "target_points": len(target),
        "covered": covered,
        "recall": recall,
        "precision": precision,
        "f1": f1,
        "interference": len(noise),
        "resisted": resisted,
        "resistance": resisted / len(noise) if noise else None,
        "update_hits": update_hits,
        "update_total": update_total,
        "update_rate": update_hits / update_total if update_total else None,
    }
