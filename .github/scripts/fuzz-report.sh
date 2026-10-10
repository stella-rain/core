#!/usr/bin/env bash
# Turns the crash artifacts of a failed long fuzz run into issues (fuzz-long.yml, ADR-020).
#
# Usage: fuzz-report.sh <dir>   <dir> holds one folder per failed target, named
#                               fuzz-crash-<target>, each with the crash inputs.
# Needs `gh`, GH_TOKEN with issues: write, and the GITHUB_* variables of a workflow run.
#
# Core is public: an issue says that a crash exists, which target, which commit, the run, and
# each input's size and SHA-256. It never contains an input, a stage or a message from the run.
# One open issue per target: a later failure adds a comment to it.
set -eu

dir=${1:?usage: fuzz-report.sh <dir>}
run_url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID"

# Opens the issue with the title, or comments on the open one that already has it.
report() {
  local title=$1 body=$2 open
  open=$(gh issue list --repo "$GITHUB_REPOSITORY" --state open --search "\"$title\" in:title" \
    --json number --jq '.[0].number // empty')
  if [ -n "$open" ]; then
    gh issue comment "$open" --repo "$GITHUB_REPOSITORY" --body "$body"
  else
    gh issue create --repo "$GITHUB_REPOSITORY" --title "$title" --body "$body"
  fi
}

found=0
for folder in "$dir"/fuzz-crash-*/; do
  [ -d "$folder" ] || continue
  found=1
  target=$(basename "$folder")
  target=${target#fuzz-crash-}
  files=""
  for input in "$folder"*; do
    [ -f "$input" ] || continue
    size=$(wc -c < "$input" | tr -d ' ')
    sum=$(sha256sum "$input" | cut -d' ' -f1)
    files="$files- \`$(basename "$input")\`: $size bytes, SHA-256 \`$sum\`
"
  done
  report "Fuzz crash: $target" "The long fuzz run found an input that makes \`$target\` fail.

- Commit: \`$GITHUB_SHA\`
- Run: $run_url
- Inputs (the run's artifact \`fuzz-crash-$target\` keeps them for 14 days):
$files
Reproduce with \`cargo +nightly fuzz run $target <input>\`. The fix goes in with the input in \`fuzz/regressions/$target/\`."
done

if [ "$found" = 0 ]; then
  report "Long fuzz run failed" "The long fuzz run failed without leaving a crash input (build, setup or timeout).

- Commit: \`$GITHUB_SHA\`
- Run: $run_url"
fi
