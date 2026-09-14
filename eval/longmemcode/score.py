"""Deterministic LongMemCode scoring (docs/METHODOLOGY.md)."""

from __future__ import annotations

CATEGORY_WEIGHT = {
    "Completion": 0.28,
    "BugFix": 0.18,
    "Refactor": 0.10,
    "TestGen": 0.08,
    "FeatureAdd": 0.08,
    "ApiDiscovery": 0.15,
    "ControlFlow": 0.05,
    "Config": 0.04,
    "Safety-net": 0.04,
}

# v1 cam cannot answer these honestly (no hops / no impl edges).
ONE_HOP_SKIP_SUBTYPES = frozenset(
    {
        "override_detection",
        "multi_hop_impact",
        "trait_implementors",
        "plugin_extension_point",
    }
)


def score_expected(expected: dict, returned: list[str]) -> float:
    kind = expected.get("kind")
    if kind == "exact_symbol":
        sid = expected.get("stable_id")
        return 1.0 if returned and returned[0] == sid else 0.0
    if kind == "in_top_k":
        sid = expected.get("stable_id")
        k = int(expected.get("k") or 5)
        return 1.0 if sid in returned[:k] else 0.0
    if kind == "exact_set":
        gold = set(expected.get("stable_ids") or [])
        got = set(returned)
        if not gold and not got:
            return 1.0
        if not gold or not got:
            return 0.0
        p = len(gold & got) / len(got)
        r = len(gold & got) / len(gold)
        return 0.0 if p + r == 0 else 2 * p * r / (p + r)
    if kind == "contains":
        req = expected.get("required") or []
        if not req:
            return 1.0
        got = set(returned)
        return len([x for x in req if x in got]) / len(req)
    return 0.0


def summarize(rows: list[dict]) -> dict:
    by_cat: dict[str, list[float]] = {}
    by_op: dict[str, list[float]] = {}
    by_sub: dict[str, list[float]] = {}
    supported: list[float] = []
    skipped: list[float] = []
    for row in rows:
        s = row["score"]
        by_cat.setdefault(row["category"], []).append(s)
        by_op.setdefault(row["op"], []).append(s)
        by_sub.setdefault(row["sub_type"], []).append(s)
        if row["sub_type"] in ONE_HOP_SKIP_SUBTYPES:
            skipped.append(s)
        else:
            supported.append(s)

    def avg(xs: list[float]) -> float:
        return sum(xs) / len(xs) if xs else 0.0

    cat_avg = {k: avg(v) for k, v in sorted(by_cat.items())}
    weight_sum = 0.0
    weighted = 0.0
    for cat, a in cat_avg.items():
        w = CATEGORY_WEIGHT.get(cat, 0.0)
        if w and by_cat[cat]:
            weighted += a * w
            weight_sum += w
    headline = weighted / weight_sum if weight_sum else 0.0
    return {
        "n": len(rows),
        "raw_accuracy": avg([r["score"] for r in rows]),
        "weighted_accuracy": headline,
        "supported_n": len(supported),
        "supported_accuracy": avg(supported),
        "deferred_n": len(skipped),
        "deferred_accuracy": avg(skipped),
        "by_category": {k: {"n": len(by_cat[k]), "avg": cat_avg[k]} for k in cat_avg},
        "by_op": {k: {"n": len(v), "avg": avg(v)} for k, v in sorted(by_op.items())},
        "by_sub_type": {
            k: {"n": len(v), "avg": avg(v)} for k, v in sorted(by_sub.items())
        },
    }
