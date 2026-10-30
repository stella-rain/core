"""Tests for the release workflows. Run: python3 -m unittest discover -s .github/scripts

Text checks, no YAML library: the workflows are read as lines. Candidate runs on main and stamps
the version into a release commit when the final tag is not Cargo.toml's (release-flow plan 9).
"""

import os
import re
import unittest

WORKFLOWS = os.path.join(os.path.dirname(__file__), "..", "workflows")


def read(name):
    with open(os.path.join(WORKFLOWS, name), encoding="utf-8") as f:
        return f.read()


def job_block(text, job_id):
    m = re.search(rf"^  {re.escape(job_id)}:\n((?:(?:   .*)?\n)*)", text, re.M)
    assert m, f"no job {job_id}"
    return m.group(1)


class CandidateRunsOnMain(unittest.TestCase):
    def setUp(self):
        self.text = read("candidate.yml")
        self.check = job_block(self.text, "check")

    def test_it_refuses_unless_the_ref_is_main(self):
        self.assertIn('release.py check-main "$GITHUB_REF"', self.check)

    def test_it_refuses_nothing_to_release_and_an_existing_candidate(self):
        self.assertIn("release.py check-release", self.check)

    def test_it_needs_no_candidate_branch(self):
        self.assertNotIn("candidate/", self.text)
        self.assertNotIn("check-candidate", self.text)
        with self.assertRaises(AssertionError):
            job_block(self.text, "cleanup")

    def test_it_asks_for_the_final_tag_of_the_floor_rule(self):
        self.assertIn("release.py final", self.check)
        self.assertNotIn("release.py version", self.text)

    def test_the_run_keeps_its_commit_when_main_moves(self):
        self.assertNotIn("moved from", self.text)


class CandidateStamps(unittest.TestCase):
    def setUp(self):
        self.text = read("candidate.yml")
        self.check = job_block(self.text, "check")
        self.build = job_block(self.text, "build")
        self.tag = job_block(self.text, "tag")

    def test_check_stamps_and_publishes_the_tested_tree(self):
        self.assertIn('release.py stamp "$final"', self.check)
        self.assertRegex(self.check, r"tree=\$\(git write-tree\)")

    def test_the_binaries_are_built_from_the_stamped_tree(self):
        stamp = self.build.index("release.py stamp")
        self.assertLess(stamp, self.build.index("cargo build"))
        self.assertIn("needs.check.outputs.tree", self.build)

    def test_the_build_refuses_a_tree_other_than_the_tested_one(self):
        self.assertIn("git write-tree", self.build)

    def test_the_release_commit_has_the_main_commit_as_its_parent(self):
        self.assertIn("parents: [$parent]", self.tag)
        self.assertIn('--arg parent "$GITHUB_SHA"', self.tag)
        self.assertIn('message: $msg', self.tag)
        self.assertIn('"Release $FINAL"', self.tag)

    def test_the_tag_job_refuses_a_tree_other_than_the_tested_one(self):
        self.assertRegex(self.tag, r'if \[ "\$tree" != "\$TREE" \]')

    def test_the_tag_job_runs_none_of_the_repositorys_code(self):
        self.assertNotIn("actions/checkout", self.tag)
        self.assertNotIn("release.py", self.tag)

    def test_the_rc_branch_points_at_the_tagged_commit(self):
        self.assertIn('ref="refs/heads/rc/$RC" -f sha="$commit"', self.tag)
        self.assertIn('-f object="$commit"', self.tag)


class CandidateKeepsTheDeepTier(unittest.TestCase):
    """The deep tier ran on every candidate/** push and so gated Candidate through its check run;
    with no such branch, Candidate calls it."""

    def test_candidate_calls_deep_and_the_tag_job_waits_for_it(self):
        text = read("candidate.yml")
        self.assertIn("uses: ./.github/workflows/deep.yml", text)
        self.assertRegex(job_block(text, "tag"), r"needs: \[check, [^\]]*deep")

    def test_deep_can_be_called_and_has_no_candidate_branch_trigger(self):
        text = read("deep.yml")
        self.assertIn("workflow_call:", text)
        self.assertNotIn("candidate/", text)


class RemovedWorkflows(unittest.TestCase):
    def test_auto_candidate_is_gone(self):
        self.assertFalse(os.path.exists(os.path.join(WORKFLOWS, "auto-candidate.yml")))

    def test_the_guard_watches_rc_branches_only(self):
        text = read("candidate-guard.yml")
        self.assertIn('branches: ["rc/**"]', text)
        self.assertNotIn("candidate/", text)


class PromoteTagsTheStampedCommit(unittest.TestCase):
    def test_promote_compares_the_version_at_the_tag_and_tags_that_commit(self):
        text = read("promote.yml")
        self.assertIn("release.py check-promote", text)
        self.assertIn('-f object="$COMMIT"', text)


class PromoteTellsTheApp(unittest.TestCase):
    def setUp(self):
        self.text = read("promote.yml")
        self.notify = job_block(self.text, "notify")
        self.tag = job_block(self.text, "tag")
        self.check = job_block(self.text, "check")

    def test_notify_waits_for_the_release(self):
        self.assertRegex(self.notify, r"needs:\s*\[check, tag\]")

    def test_notify_runs_none_of_the_repositorys_code(self):
        self.assertNotIn("actions/checkout", self.notify)
        self.assertNotIn("release.py", self.notify)

    def test_notify_can_only_read_this_repository(self):
        self.assertIn("contents: read", self.notify)
        self.assertNotIn("contents: write", self.notify)
        self.assertRegex(self.text, re.compile(r"^permissions: \{\}$", re.M))

    def test_the_token_is_the_bridge_apps_for_app_only(self):
        for line in (
            "actions/create-github-app-token@v3",
            "client-id: ${{ vars.BRIDGE_APP_CLIENT_ID }}",
            "private-key: ${{ secrets.BRIDGE_APP_PRIVATE_KEY }}",
            "repositories: app",
            "permission-contents: write",
        ):
            self.assertIn(line, self.notify)
        self.assertEqual(re.findall(r"permission-[a-z-]+", self.notify), ["permission-contents"])

    def test_the_tag_job_never_sees_the_app_key(self):
        for block in (self.tag, self.check):
            self.assertNotIn("BRIDGE_APP", block)
            self.assertNotIn("create-github-app-token", block)

    def test_it_sends_core_release_with_the_tag(self):
        self.assertIn("event_type=core-release", self.notify)
        self.assertIn("client_payload[tag]=$TAG", self.notify)
        self.assertIn("repos/$GITHUB_REPOSITORY_OWNER/app/dispatches", self.notify)

    def test_the_tag_comes_through_env_never_inline(self):
        self.assertIn("TAG: ${{ needs.check.outputs.final }}", self.notify)
        in_script = False
        script_indent = 0
        for line in self.notify.splitlines():
            indent = len(line) - len(line.lstrip())
            if in_script and line.strip() and indent <= script_indent:
                in_script = False
            if in_script:
                self.assertNotIn("${{", line, "expression inside a run script")
            elif re.match(r"^\s+(- )?run: \|\s*$", line):
                in_script = True
                script_indent = indent
            if "${{" in line and not in_script:
                self.assertRegex(line, r"^\s+(- )?[A-Za-z_-]+: .*\$\{\{")

    def test_only_a_final_release_is_announced(self):
        self.assertIn("^v[0-9]+\\.[0-9]+\\.[0-9]+$", self.notify)
        self.assertIn("isPrerelease", self.notify)
        self.assertIn("isDraft", self.notify)
        self.assertLess(
            self.notify.index("isPrerelease"),
            self.notify.index("actions/create-github-app-token"),
        )

    def test_a_failed_dispatch_fails_the_run(self):
        self.assertNotIn("continue-on-error", self.notify)
        self.assertNotIn("|| true", self.notify)
        self.assertGreater(
            self.notify.rindex("::error::"), self.notify.index("event_type=core-release")
        )

    def test_the_release_steps_are_untouched(self):
        self.assertIn("contents: write", self.tag)
        self.assertIn("gh release create", self.tag)
        self.assertIn("--latest", self.tag)
        self.assertNotIn("core-release", self.tag)


if __name__ == "__main__":
    unittest.main()
