#!/usr/bin/env python3
"""Scorer checks. No model calls."""

from __future__ import annotations

import unittest

from score import score_bank


def memory(mem_id: str, body: str, supersedes: str | None = None) -> dict:
    return {"id": mem_id, "summary": mem_id, "body": body, "supersedes_id": supersedes}


class ScoreTests(unittest.TestCase):
    def test_a_covered_fact_and_a_missed_fact(self):
        points = [
            {"memory_content": "Martin is considering a career change", "memory_source": "primary", "is_update": "False"},
            {"memory_content": "Martin plays the violin every evening", "memory_source": "primary", "is_update": "False"},
        ]
        scored = score_bank([memory("m1", "Martin is considering a career change for his health")], points)
        self.assertEqual(scored["covered"], 1)
        self.assertAlmostEqual(scored["recall"], 0.5)

    def test_an_update_counts_only_when_supersedes_points_at_the_old_memory(self):
        points = [
            {
                "memory_content": "Martin wants to leave Huaxin Consulting",
                "memory_source": "primary",
                "is_update": "True",
                "original_memories": ["Martin works at Huaxin Consulting as a director"],
            }
        ]
        linked = [
            memory("old", "Martin works at Huaxin Consulting as a director"),
            memory("new", "Martin wants to leave Huaxin Consulting", "old"),
        ]
        unlinked = [
            memory("old", "Martin works at Huaxin Consulting as a director"),
            memory("new", "Martin wants to leave Huaxin Consulting"),
        ]
        self.assertEqual(score_bank(linked, points)["update_hits"], 1)
        self.assertEqual(score_bank(unlinked, points)["update_hits"], 0)

    def test_interference_that_was_not_stored_counts_as_resisted(self):
        points = [
            {
                "memory_content": "Martin has a brother named Oscar",
                "memory_source": "interference",
                "is_update": "False",
            }
        ]
        scored = score_bank([memory("m1", "Martin is reflecting on his career")], points)
        self.assertEqual(scored["resistance"], 1.0)
        self.assertEqual(scored["target_points"], 0)


if __name__ == "__main__":
    unittest.main()
