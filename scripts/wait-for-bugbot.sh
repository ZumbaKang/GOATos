#!/usr/bin/env bash
# Waits for the "Cursor Bugbot" GitHub check to finish on a given commit,
# and exits non-zero if Bugbot found unresolved issues - so the auto-merge
# workflow never ships a PR Bugbot has flagged, even before a human (or
# Bugbot's own Autofix) has had a chance to see or act on it.
#
# Bugbot's check defaults to a `neutral` conclusion when it finds issues
# (not `failure`) - see https://cursor.com/docs/bugbot - so this treats
# anything other than `success` as blocking, rather than relying on GitHub
# branch protection to enforce it (branch protection alone would let
# `neutral` findings through).
#
# If no "Cursor Bugbot" check ever appears, this fails *open* after a grace
# period: the point is to respect Bugbot when it's enabled for this repo,
# not to require it to be enabled.
#
# Requires: `gh` authenticated (GH_TOKEN), `jq`.
# Env vars: REPO (owner/repo), SHA (commit SHA to check).
# Optional: GRACE_PERIOD_SECONDS, MAX_WAIT_SECONDS, POLL_INTERVAL_SECONDS.
set -uo pipefail

REPO="${REPO:?REPO env var required, e.g. owner/repo}"
SHA="${SHA:?SHA env var required}"
CHECK_NAME="Cursor Bugbot"
GRACE_PERIOD_SECONDS="${GRACE_PERIOD_SECONDS:-90}"
MAX_WAIT_SECONDS="${MAX_WAIT_SECONDS:-600}"
POLL_INTERVAL_SECONDS="${POLL_INTERVAL_SECONDS:-15}"

elapsed=0
seen=false

while [ "$elapsed" -lt "$MAX_WAIT_SECONDS" ]; do
  run_json=$(gh api "repos/$REPO/commits/$SHA/check-runs" --jq \
    "[.check_runs[] | select(.name == \"$CHECK_NAME\")] | sort_by(.started_at) | last // empty")

  if [ -n "$run_json" ]; then
    seen=true
    status=$(echo "$run_json" | jq -r '.status')
    conclusion=$(echo "$run_json" | jq -r '.conclusion // "pending"')

    if [ "$status" = "completed" ]; then
      echo "'$CHECK_NAME' completed with conclusion: $conclusion"
      if [ "$conclusion" = "success" ]; then
        echo "Bugbot found no unresolved issues - OK to merge."
        exit 0
      else
        echo "Bugbot reported '$conclusion' - NOT merging."
        echo "Review its comments on the PR (and consider enabling Autofix in the Bugbot dashboard: https://cursor.com/dashboard/bugbot)."
        exit 1
      fi
    fi

    echo "[${elapsed}s] '$CHECK_NAME' status: $status (waiting for it to complete)"
  elif [ "$elapsed" -ge "$GRACE_PERIOD_SECONDS" ]; then
    echo "No '$CHECK_NAME' check appeared within ${GRACE_PERIOD_SECONDS}s - assuming Bugbot isn't enabled for this repo. Proceeding."
    exit 0
  else
    echo "[${elapsed}s] No '$CHECK_NAME' check yet (grace period: ${GRACE_PERIOD_SECONDS}s)"
  fi

  sleep "$POLL_INTERVAL_SECONDS"
  elapsed=$((elapsed + POLL_INTERVAL_SECONDS))
done

if [ "$seen" = true ]; then
  echo "'$CHECK_NAME' did not complete within ${MAX_WAIT_SECONDS}s - not merging (timed out waiting for Bugbot)."
  exit 1
else
  echo "'$CHECK_NAME' never appeared within ${MAX_WAIT_SECONDS}s - proceeding without it."
  exit 0
fi
