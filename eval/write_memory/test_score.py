#!/usr/bin/env python3
"""Scorer checks. No model calls."""

from __future__ import annotations

import json
import unittest
from pathlib import Path

from score import score_case

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]


def memory(mem_id: str, body: str, links: list[str] | None = None) -> dict:
    return {"id": mem_id, "summary": mem_id, "body": body, "links": links or []}


class ScoreTests(unittest.TestCase):
    def test_one_body_covers_facts_without_splitting_them(self):
        case = json.loads((HERE / "cases.json").read_text())[0]
        packed = memory("all", case["note"])
        scored = score_case([packed], case)
        self.assertEqual(scored["covered"], 7)
        self.assertEqual(scored["exclusive"], 0)
        self.assertEqual(scored["link_hits"], scored["link_total"])

    def test_split_memories_are_exclusive_and_links_count(self):
        case = json.loads((HERE / "cases.json").read_text())[2]
        memories = [
            memory("a", "The constant k is 60, and the fused score is scaled by k+1.", ["b"]),
            memory("b", "A successful recall sets S to S * 1.7. needs_update is below 0.3.", ["a"]),
        ]
        scored = score_case(memories, case)
        self.assertEqual(scored["exclusive"], 2)
        self.assertEqual(scored["link_hits"], 1)

    def test_a_solution_path_split_across_memories_is_not_exclusive(self):
        case = json.loads((HERE / "cases.json").read_text())[1]
        memories = [
            memory("cause", "Hub restore never loads the LSP manager."),
            memory("fix", "skip the PATH check and hide the install-failure toast"),
        ]
        scored = score_case(memories, case)
        self.assertEqual(scored["exclusive"], 0)

    def test_writer_rule_matches_the_mcp_instructions(self):
        instructions = (REPO / "src" / "mcp.rs").read_text()
        rule = (HERE / "run.py").read_text()
        sentence = "If two pieces can be asked about separately, or can become outdated separately"
        self.assertIn(sentence, instructions)
        self.assertIn(sentence, rule)


if __name__ == "__main__":
    unittest.main()
