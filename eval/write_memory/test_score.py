#!/usr/bin/env python3
"""Scorer checks. No model calls."""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(HERE))

from cases import ensure_cases  # noqa: E402
from score import score_case  # noqa: E402


def memory(mem_id: str, body: str, links: list[str] | None = None) -> dict:
    return {"id": mem_id, "summary": mem_id, "body": body, "links": links or []}


def cases() -> list[dict]:
    return json.loads(ensure_cases().read_text())


class ScoreTests(unittest.TestCase):
    def test_one_body_covers_facts_without_splitting_them(self):
        case = next(case for case in cases() if case["id"] == "recall-pipeline")
        packed = memory("all", case["note"])
        scored = score_case([packed], case)
        self.assertEqual(scored["covered"], len(case["facts"]))
        self.assertEqual(scored["exclusive"], 0)
        self.assertEqual(scored["link_hits"], scored["link_total"])

    def test_split_memories_are_exclusive_and_links_count(self):
        case = next(case for case in cases() if case["id"] == "recall-and-forgetting")
        memories = []
        for index, fact in enumerate(case["facts"]):
            other = 1 - index
            memories.append(memory(f"m{index}", " ".join(fact["must"]), [f"m{other}"]))
        scored = score_case(memories, case)
        self.assertEqual(scored["exclusive"], len(case["facts"]))
        self.assertEqual(scored["link_hits"], 1)

    def test_a_solution_path_split_across_memories_is_not_exclusive(self):
        case = next(case for case in cases() if case["expect"] == "keep")
        must = case["facts"][0]["must"]
        memories = [
            memory("cause", must[0]),
            memory("fix", " ".join(must[1:])),
        ]
        scored = score_case(memories, case)
        self.assertEqual(scored["covered"], 0)
        self.assertEqual(scored["exclusive"], 0)

    def test_writer_rule_matches_the_mcp_instructions(self):
        instructions = (REPO / "src" / "mcp.rs").read_text()
        rule = (HERE / "run.py").read_text()
        sentence = "If two pieces can be asked about separately, or can become outdated separately"
        self.assertIn(sentence, instructions)
        self.assertIn(sentence, rule)


if __name__ == "__main__":
    unittest.main()
