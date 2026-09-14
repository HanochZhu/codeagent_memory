import unittest

from score import score_expected, summarize
from scip import catalog_from_scenarios, index_catalog, path_matches, trailing_ident


class ScipTests(unittest.TestCase):
    def test_trailing_ident(self):
        sid = "rust-analyzer cargo clap_builder 4.6.0 builder/command/Command#"
        self.assertEqual(trailing_ident(sid), "Command")
        self.assertEqual(
            trailing_ident("rust-analyzer cargo clap 4.6.1 impl#[DerivedArgs][Args]augment_args()."),
            "augment_args",
        )

    def test_catalog(self):
        cat = index_catalog(
            [
                "rust-analyzer cargo clap 4.6.1 type_alias_regressions/Command#DoSomething#",
                "file:clap_mangen/tests/testsuite/main.rs",
            ]
        )
        self.assertIn("DoSomething", cat)
        self.assertTrue(cat["DoSomething"][0].endswith("DoSomething#"))

    def test_path_matches_scip_module(self):
        self.assertTrue(
            path_matches("clap_builder/src/builder/command.rs", "builder/command/Command")
        )
        self.assertTrue(
            path_matches(
                "clap_builder/src/builder/value_parser.rs",
                "builder/value_parser",
            )
        )
        self.assertFalse(path_matches("examples/find.rs", "builder/value_parser/Value"))

    def test_catalog_harvests_query_name_for_local_ids(self):
        cat = catalog_from_scenarios(
            [
                {
                    "query": {"op": "lookup", "name": "F", "bare_name": True},
                    "expected": {"kind": "contains", "required": ["local 87"]},
                }
            ]
        )
        self.assertIn("local 87", cat["F"])


class ScoreTests(unittest.TestCase):
    def test_contains(self):
        exp = {"kind": "contains", "required": ["a", "b"]}
        self.assertEqual(score_expected(exp, ["a", "b", "c"]), 1.0)
        self.assertEqual(score_expected(exp, ["a"]), 0.5)
        self.assertEqual(score_expected(exp, []), 0.0)

    def test_exact_set_empty(self):
        exp = {"kind": "exact_set", "stable_ids": []}
        self.assertEqual(score_expected(exp, []), 1.0)

    def test_summarize_splits_deferred(self):
        rows = [
            {
                "category": "Completion",
                "sub_type": "lookup_class_by_name",
                "op": "lookup",
                "score": 1.0,
            },
            {
                "category": "BugFix",
                "sub_type": "multi_hop_impact",
                "op": "callers",
                "score": 0.0,
            },
        ]
        s = summarize(rows)
        self.assertEqual(s["supported_n"], 1)
        self.assertEqual(s["deferred_n"], 1)
        self.assertEqual(s["supported_accuracy"], 1.0)


if __name__ == "__main__":
    unittest.main()
