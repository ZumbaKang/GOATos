---
name: bugbot-and-code-review
description: How Cursor Bugbot's automated PR review integrates with this repo's CI/auto-merge, how to enable it and its Autofix feature (dashboard-only steps), and what to do when auto-merge is blocked by a Bugbot finding. Use this when a PR's auto-merge is blocked with a "Cursor Bugbot" comment, or when setting up/adjusting Bugbot for this repo.
---

# Bugbot and code review

[Bugbot](https://cursor.com/docs/bugbot) is Cursor's automated PR reviewer.
When enabled for a repo, it reviews every PR (and re-reviews on every push),
leaving inline comments on issues it finds, each with "Fix in Cursor"/"Fix
in Web" links. It also posts a GitHub check named **`Cursor Bugbot`** on the
PR's head commit.

## How this repo's auto-merge respects Bugbot

`.github/workflows/ci.yml`'s `auto-merge` job runs
[`scripts/wait-for-bugbot.sh`](../../../scripts/wait-for-bugbot.sh) before
ever merging a PR:

- It polls for the `Cursor Bugbot` check on the PR's head SHA and waits for
  it to complete.
- **Important gotcha**: Bugbot's check conclusion defaults to `neutral` when
  it finds issues, *not* `failure` - so simply requiring the check via
  GitHub branch protection would not actually block a merge. This script
  treats a completed non-`success` conclusion as blocking *when Bugbot
  actually reviewed*, regardless of the literal conclusion string.
- **Also fail-open when Bugbot never reviewed.** A usage/spend-limit (or
  similar) failure still posts a completed check with conclusion `neutral`
  and an `output.title` of `Error` / text like "couldn't run - usage limit
  reached". That is *not* a finding - treating it as one strands every PR
  until a human raises the quota. The script detects that shape and
  proceeds, same as when the check never appears.
- If found and blocking, the merge is skipped and a comment explaining why
  is posted on the PR (see the `auto-merge` job's "Comment if blocked by
  Bugbot" step).
- If the check never appears at all (a short grace period, then it gives
  up), the merge proceeds anyway - this fails *open* so auto-merge still
  works for this repo even if Bugbot isn't enabled, rather than silently
  requiring a feature that has to be turned on separately (see below).

## Enabling Bugbot for this repo (one-time, dashboard-only)

This can't be done from a repo file or by an agent - it's a per-repo toggle
in the Cursor dashboard, done by whoever administers this project:

1. Make sure the Cursor GitHub App is connected (**Cursor Dashboard →
   Integrations**, [cursor.com/dashboard/integrations](https://cursor.com/dashboard/integrations))
   - it's the same connection Cloud Agents use, so if automations are
   already opening PRs against this repo, this step is likely already done.
2. Go to the [**Bugbot dashboard**](https://cursor.com/dashboard/bugbot) and
   enable it for this repository specifically (connecting the GitHub App
   does not automatically turn Bugbot on for every repo).
3. **Also enable Autofix** (same dashboard page) and set it to **"Create
   New Branch"** (recommended over committing to the existing branch). This
   is the actual "go back and fix it automatically" mechanism: when Bugbot
   finds an issue, Autofix spawns a Cloud Agent that analyzes the finding,
   pushes a fix, and comments on the original PR - no separate Automation
   needs to be built for this, it's a built-in Bugbot feature. It requires
   on-demand usage and Storage to be enabled on the account/team.

Without step 3, a blocked PR (per the CI gate above) will sit there until a
human clicks "Fix in Cursor"/"Fix in Web" on one of Bugbot's comments, or
manually fixes and pushes - which still works, just isn't fully automated.

## If you (or an agent) land on a PR blocked by Bugbot

1. Read Bugbot's inline comments on the PR - they include an explanation
   and a suggested fix for each finding.
2. If Autofix (above) is enabled, it may already be working on this, or may
   have opened a separate fix branch/PR - check for that before duplicating
   the work.
3. Otherwise, fix the specific issues Bugbot flagged (only those - per the
   `roadmap-automation` skill's scope discipline, don't scope-creep into
   unrelated changes), push to the same branch, and CI will re-run
   automatically: `wait-for-bugbot.sh` re-checks the new commit's `Cursor
   Bugbot` check, and merges once it comes back clean.
4. Do not attempt to bypass this by disabling the check or merging manually
   over a real finding - if Bugbot is wrong about something, resolve the
   thread/comment on GitHub (which the next check run takes into account)
   rather than working around the automation.
