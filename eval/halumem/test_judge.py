#!/usr/bin/env python3
"""Judge aggregation checks. No model calls."""

from __future__ import annotations

import unittest

from judge import aggregate
from llm import parse_json_object


def integrity(source: str, score: int, importance: float = 1) -> dict:
    return {
        "memory_source": source,
        "importance": importance,
        "memory_integrity_score": score,
    }


class JudgeTests(unittest.TestCase):
    def test_recall_counts_only_a_full_score(self):
        scored = aggregate(
            {
                "memory_integrity_records": [
                    integrity("primary", 2),
                    integrity("primary", 1),
                    integrity("interference", 0),
                ],
                "memory_accuracy_records": [
                    {"memory_accuracy_score": 2, "is_included_in_golden_memories": "true"},
                ],
                "memory_update_records": [{"memory_update_type": "Correct"}],
                "question_answering_records": [{"result_type": "Omission"}],
            }
        )
        self.assertAlmostEqual(scored["recall"], 0.5)
        self.assertAlmostEqual(scored["weighted_recall"], 0.75)
        self.assertAlmostEqual(scored["target_accuracy"], 1.0)
        self.assertAlmostEqual(scored["f1"], 2 / 3)
        self.assertAlmostEqual(scored["resistance"], 1.0)
        self.assertAlmostEqual(scored["update_correct"], 1.0)
        self.assertAlmostEqual(scored["qa_correct"], 0.0)
        self.assertAlmostEqual(scored["qa_omission"], 1.0)

    def test_a_missing_judge_score_stays_in_the_denominator(self):
        scored = aggregate(
            {
                "memory_integrity_records": [integrity("primary", None)],
                "memory_accuracy_records": [],
                "memory_update_records": [],
                "question_answering_records": [],
            }
        )
        self.assertEqual(scored["recall"], 0.0)
        self.assertEqual(scored["counts"]["integrity_valid"], 0)
        self.assertIsNone(scored["update_correct"])
        self.assertIsNone(scored["qa_correct"])

    def test_json_object_parses_from_a_fence_or_bare_text(self):
        self.assertEqual(parse_json_object('```json\n{"score": "2"}\n```')["score"], "2")
        self.assertEqual(parse_json_object('note {"score": 1} tail')["score"], 1)


if __name__ == "__main__":
    unittest.main()
