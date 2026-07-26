---
name: roadmap-automation
description: The exact procedure for an automated agent run to pick the next unchecked task from ROADMAP.md, implement it, and open a PR that GitHub Actions will test and auto-merge. Use this whenever asked to "work on the next roadmap task", "continue the roadmap", or when running as a scheduled/triggered Cursor Automation for this repo.
---

# Roadmap automation procedure

This repo is meant to build itself out incrementally, with minimal human
intervention: an automation (see "Setting up the Automation" below) runs
this procedure on a schedule/trigger, and CI (`.github/workflows/ci.yml`)
auto-merges the result if it passes. Follow these steps in order.

## 1. Pick exactly one task

Open [`ROADMAP.md`](../../../ROADMAP.md) and find the **first** unchecked
task (a line starting with `- [ ]`), in file order - phases and tasks are
already sequenced by dependency, so don't skip ahead even if a later task
looks easier or more interesting.

Do **one** task per run. Small, reviewable, independently-mergeable PRs are
the entire point of this setup - resist the urge to bundle multiple tasks
into one PR "while you're in there."

If there are no unchecked tasks left in `ROADMAP.md`, stop and say so
(don't invent new work) - that's a signal for a human to add a new phase.

## 2. Read the relevant skill

Each phase in `ROADMAP.md` links to a skill under `.cursor/skills/` (e.g.
`interrupts-and-exceptions`, `memory-management`, `drivers`). Read it before
writing any code - it has the conventions, gotchas, and suggested approach
for that subsystem, including lessons from past mistakes that are not
worth re-learning.

## 3. Implement the task

- Create a branch following this repo's `cursor/<descriptive-name>`
  convention (the exact suffix is assigned automatically per agent run -
  don't hardcode one from a past session).
- Implement *only* what the task describes. If you discover the task needs
  to be split further once you're in the code, do the smallest coherent
  slice of it and leave the rest as a new `ROADMAP.md` task rather than
  scope-creeping the current PR.
- Follow existing conventions (driver shape in `kernel/src/vga.rs`/
  `serial.rs`, the defensive/non-panicking error handling pattern, doc
  comments on `unsafe fn`, etc.) - see `.cursor/skills/drivers/SKILL.md`
  and `.cursor/skills/qemu-testing-and-verification/SKILL.md`.

## 4. Verify against the task's "Done when" criteria

At minimum:

```bash
cd kernel && cargo clippy -- -D warnings
cd .. && make test
```

`make test` runs `scripts/ci-test.sh` - the same headless boot check CI
runs. If the task's "Done when" criteria in `ROADMAP.md` describe something
`make test` can't check (a visual VGA change, keyboard/mouse input, a new
exception being triggered on purpose, etc.), also do the manual/visual
verification described in `.cursor/skills/qemu-testing-and-verification/`
(a real QEMU screendump) - and if the change touches boot-time BIOS calls,
disk I/O, or anything timing-sensitive, spot-check it against the browser
demo too per `.cursor/skills/web-demo-packaging/` (several real bugs so far
only showed up under v86, not QEMU).

Do not open a PR for something you haven't actually verified boots
correctly - CI will catch a totally broken boot, but it won't catch "looks
right but the VGA output is garbled," for example.

## 5. Update the roadmap (and README, if a phase just finished)

- Check off the task in `ROADMAP.md` (`- [ ]` -> `- [x]`) in the same PR.
- If that was the **last** task in its phase, also check off the matching
  item in the README's `## Status` list.

## 6. Commit, push, and open a PR

- Commit message: reference the task, e.g.
  `Phase 1.1: add kernel-owned GDT`.
- Push the branch and open a PR against `main`.
- Write a PR description that states which task this is, links to the
  `ROADMAP.md` entry, and briefly says how it was verified (step 4).
- **A draft PR is fine.** The Cursor Automation's PR tool has no draft
  option and creates drafts, so `ci.yml`'s `ready-for-review` job marks
  `cursor/*` PRs in this repo ready automatically. Don't try to flip it by
  hand - just don't count on the PR still being a draft a minute later.

From here, `.github/workflows/ci.yml` takes over: it takes the PR out of
draft, builds the kernel, runs `cargo clippy`, runs the boot test, and -
only for `cursor/*` branches in this repository, and only if all of that
passes - merges the PR automatically (squash + delete branch). If CI fails,
the PR is left open for the next run (or a human) to fix; **never bypass a
failing check to force a merge.**

## Setting up the Automation itself

This skill describes what an agent run should *do*; it doesn't create the
schedule/trigger that kicks off that run - that's a
[Cursor Automation](https://cursor.com/automations), configured through the
Cursor dashboard (or the `/automate` skill in a local chat session), not a
repo file. When setting one up for this repo:

- **Repository**: this one (`GOATos`).
- **Prompt**: something like *"Follow the procedure in
  `.cursor/skills/roadmap-automation/SKILL.md` to pick up and complete the
  next unchecked task in ROADMAP.md."*
- **Trigger**: a `Pull request merged` trigger (on `main`) creates a tight
  loop - each merge immediately kicks off the run that picks the next task
  - optionally combined with a scheduled trigger (e.g. daily) as a
  fallback in case the chain ever stalls (a failed/blocked PR, no
  unchecked tasks left, etc.).
- Make sure "Pull request creation" is enabled for the automation (it is
  by default). Whether it creates drafts doesn't matter - `ci.yml` marks
  `cursor/*` PRs ready itself, per step 6 above.
