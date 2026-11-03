"""Unit tests for scripts/adr_ids.py: the scan that keeps every ADR citation real."""

import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(__file__))

import adr_ids

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
IDS = {"SIM-19": 7}


def kinds(findings):
    return [(f[1], f[2], f[3]) for f in findings]


def scan(text, path="src/x.rs"):
    return adr_ids.scan_text(path, text, IDS)


class TestScan(unittest.TestCase):
    def test_a_real_id_and_point_pass(self):
        self.assertEqual(scan("// see ADR-SIM-19 point 7\n"), [])
        self.assertEqual(scan("// see ADR-SIM-19\n"), [])

    def test_an_id_the_list_lacks_is_missing(self):
        self.assertEqual(kinds(scan("// ADR-SIM-3\n")), [(1, "missing", "ADR-SIM-3")])

    def test_a_legacy_number_is_never_allowed(self):
        self.assertEqual(kinds(scan("// ADR-020\n// ADR 080\n")),
                         [(1, "legacy", "ADR-020"), (2, "legacy", "ADR 080")])

    def test_a_point_above_the_count_is_found(self):
        self.assertEqual(kinds(scan("// ADR-SIM-19 point 8\n")),
                         [(1, "point", "ADR-SIM-19 point 8")])
        found = kinds(scan("// ADR-SIM-19 points 3, 4 and 9\n"))
        self.assertEqual(found, [(1, "point", "ADR-SIM-19 point 9")])
        self.assertEqual(kinds(scan("// points 2 to 9 of ADR-SIM-19\n")),
                         [(1, "point", f"ADR-SIM-19 point {n}") for n in (8, 9)])

    def test_a_citation_wrapped_onto_the_next_comment_line(self):
        text = "//! (ADR-SIM-19 point\n//! 9)\n"
        self.assertEqual(kinds(scan(text)), [(1, "point", "ADR-SIM-19 point 9")])
        self.assertEqual(scan("//! (ADR-SIM-19 point\n//! 7)\n"), [])

    def test_a_document_path_is_found(self):
        self.assertEqual(kinds(scan("// see docs/design/combat.md\n")),
                         [(1, "document", "docs/design/combat.md")])
        self.assertEqual(kinds(scan("// docs/architecture/process.md#gates\n")),
                         [(1, "document", "docs/architecture/process.md")])

    def test_excluded_paths_are_not_read(self):
        with tempfile.TemporaryDirectory() as root:
            names = ["tests/corpus/a.txt", "tests/fixtures/b.json", "scripts/adr_ids.txt",
                     "src/d.rs"]
            for name in names:
                os.makedirs(os.path.join(root, os.path.dirname(name)), exist_ok=True)
                with open(os.path.join(root, name), "w") as f:
                    f.write("ADR-020\n")
            found = adr_ids.scan_tree(root, IDS, files=names)
            self.assertEqual([f[0] for f in found], ["src/d.rs"])


    def test_the_reason_lines_of_the_bench_ledger_are_records(self):
        text = ('# The ledger (ADR-SIM-20 point 3).\n'
                '[[accepted]]\n'
                'reason = "player index threaded through every system (ADR-SIM-12)"\n'
                'note = "ADR-SIM-12"\n')
        found = adr_ids.scan_text("benches/accepted.toml", text, IDS)
        self.assertEqual(kinds(found), [(1, "missing", "ADR-SIM-20"), (4, "missing", "ADR-SIM-12")])
        # the same `reason` line in any other file is scanned
        other = adr_ids.scan_text("benches/other.toml", text.split("\n")[2] + "\n", IDS)
        self.assertEqual(kinds(other), [(1, "missing", "ADR-SIM-12")])

    def test_a_file_entry_names_one_file_not_a_prefix(self):
        self.assertTrue(adr_ids.skipped("scripts/adr_ids.txt"))
        self.assertFalse(adr_ids.skipped("scripts/adr_ids.txt.bak"))
        for path in ("AGENTS.md", ".claude/rules/ci.md", ".agents/rules/replay.md",
                     ".github/scripts/release.py", "benches/accepted.toml"):
            self.assertFalse(adr_ids.skipped(path), path)


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def decision(points):
    body = "\n".join(f"{n}. **Point {n}.** Text.\n   continued" for n in range(1, points + 1))
    return f"# ADR\n\n## Context\n\n1. not a decision\n\n## Decision\n\n{body}\n\n## Consequences\n"


def make_app(root, with_new_sim=True):
    adr = os.path.join(root, "docs", "adr")
    for area in adr_ids.FIRST_NUMBER:
        os.makedirs(os.path.join(adr, area.lower()), exist_ok=True)
    write(os.path.join(adr, "sim", "014-old.md"), decision(9))
    write(os.path.join(adr, "svc", "007-z.md"), decision(2))
    write(os.path.join(adr, "svc", "006-old.md"), decision(4))
    if with_new_sim:
        write(os.path.join(adr, "sim", "015-y.md"), decision(3))


class TestGenerator(unittest.TestCase):
    def test_only_the_set_in_force_is_listed_sorted_with_counts(self):
        with tempfile.TemporaryDirectory() as app:
            make_app(app)
            write(os.path.join(app, "docs", "adr", "sim", "016-q.md"), decision(11))
            ids = adr_ids.read_app(app)
            self.assertEqual(ids, {"SIM-15": 3, "SIM-16": 11, "SVC-7": 2})
            lines = adr_ids.render(ids).splitlines()
            self.assertEqual(lines[0], adr_ids.HEADER)
            self.assertEqual(lines[1:], ["ADR-SIM-15 3", "ADR-SIM-16 11", "ADR-SVC-7 2"])

    def test_numbers_sort_as_numbers(self):
        text = adr_ids.render({"SIM-9": 1, "SIM-10": 2})
        self.assertEqual(text.splitlines()[1:], ["ADR-SIM-9 1", "ADR-SIM-10 2"])

    def test_a_new_file_without_decision_points_stops_the_generator(self):
        with tempfile.TemporaryDirectory() as app:
            make_app(app)
            write(os.path.join(app, "docs", "adr", "sim", "017-bad.md"),
                  "# ADR\n\n## Decision\n\nNo numbered point.\n")
            with self.assertRaises(SystemExit) as stop:
                adr_ids.read_app(app)
            self.assertIn("017-bad.md", str(stop.exception))

    def test_write_then_check_agree_and_a_change_fails_check(self):
        with tempfile.TemporaryDirectory() as app:
            make_app(app)
            out = os.path.join(app, "list.txt")
            run = lambda cmd: subprocess.run(  # noqa: E731
                [sys.executable, os.path.join(HERE, "adr_ids.py"), cmd, app, out],
                capture_output=True, text=True)
            self.assertEqual(run("write").returncode, 0)
            self.assertEqual(run("check").returncode, 0)
            write(os.path.join(app, "docs", "adr", "sim", "015-y.md"), decision(4))
            self.assertEqual(run("check").returncode, 1)

    def test_parse_list_round_trips(self):
        ids = {"SIM-19": 7, "SVC-7": 7}
        self.assertEqual(adr_ids.parse_list(adr_ids.render(ids)), ids)


class TestRealTree(unittest.TestCase):
    def test_every_citation_in_the_tree_is_real(self):
        ids = adr_ids.load_list(os.path.join(HERE, adr_ids.LIST_NAME))
        found = adr_ids.scan_tree(ROOT, ids)
        shown = "\n".join(f"{p}:{n}: {k} {t}" for p, n, k, t in found[:40])
        self.assertEqual(found, [], f"{len(found)} findings in {len({f[0] for f in found})} "
                                    f"files; first:\n{shown}")

    def test_the_list_matches_the_app_records(self):
        app = os.environ.get("STELLA_APP_DIR") or os.path.join(ROOT, "..", "app")
        if not os.path.isdir(os.path.join(app, "docs", "adr")):
            self.skipTest("NOT VERIFIED: the list was not compared with app (no app clone)")
        with open(os.path.join(HERE, adr_ids.LIST_NAME), encoding="utf-8") as f:
            self.assertEqual(f.read(), adr_ids.render(adr_ids.read_app(app)))


if __name__ == "__main__":
    unittest.main()
