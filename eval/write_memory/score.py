"""Score memories a model stored against hand-labeled facts.

A fact is covered when one memory contains every required phrase. It is
exclusive when that memory contains no other fact. A required link counts
when the two facts share a memory or their memories are linked.
"""

from __future__ import annotations


def blob(memory: dict) -> str:
    return f"{memory.get('summary') or ''}\n{memory.get('body') or ''}".casefold()


def hosts(memories: list[dict], fact: dict) -> list[dict]:
    needed = [phrase.casefold() for phrase in fact["must"]]
    return [memory for memory in memories if all(phrase in blob(memory) for phrase in needed)]


def score_case(memories: list[dict], case: dict) -> dict:
    facts = case["facts"]
    hosted = {fact["id"]: hosts(memories, fact) for fact in facts}
    covered = exclusive = 0
    for fact in facts:
        found = hosted[fact["id"]]
        if found:
            covered += 1
        others = [item for item in facts if item["id"] != fact["id"]]
        alone = [
            memory
            for memory in found
            if not any(memory in hosted[other["id"]] for other in others)
        ]
        if alone:
            exclusive += 1

    seen_pairs: set[tuple[str, str]] = set()
    link_hits = link_total = 0
    by_id = {memory["id"]: set(memory.get("links") or []) for memory in memories}
    for fact in facts:
        for other_id in fact.get("links") or []:
            pair = tuple(sorted((fact["id"], other_id)))
            if pair in seen_pairs or other_id not in hosted:
                continue
            seen_pairs.add(pair)
            link_total += 1
            left = {memory["id"] for memory in hosted[fact["id"]]}
            right = {memory["id"] for memory in hosted[other_id]}
            if left & right:
                link_hits += 1
                continue
            if any(other in by_id.get(one, ()) for one in left for other in right):
                link_hits += 1

    bodies = [len(memory.get("body") or "") for memory in memories]
    count = len(facts)
    return {
        "memories": len(memories),
        "mean_body_chars": round(sum(bodies) / len(bodies)) if bodies else 0,
        "max_body_chars": max(bodies) if bodies else 0,
        "covered": covered,
        "exclusive": exclusive,
        "facts": count,
        "coverage": covered / count if count else 0,
        "exclusive_rate": exclusive / count if count else 0,
        "link_hits": link_hits,
        "link_total": link_total,
    }


def mean_rate(rows: list[dict], key: str) -> float:
    if not rows:
        return 0.0
    return sum(row[key] for row in rows) / len(rows)
