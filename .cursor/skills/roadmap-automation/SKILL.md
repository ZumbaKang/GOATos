---
name: roadmap-automation
description: The exact procedure for an automated agent run to pick the next unchecked task from the core or GUI roadmap track, implement it, and open a PR that GitHub Actions will test and auto-merge. Use this whenever asked to "work on the next roadmap task", "continue the roadmap", or when running as a scheduled/triggered Cursor Automation for this repo.
---

# Roadmap automation procedure

This repo builds itself out on **two parallel tracks** (see
[`ROADMAP.md`](../../../ROADMAP.md)):

| `TRACK` | Task file | Typical ownership |
|---------|-----------|-------------------|
| `core` | [`ROADMAP-CORE.md`](../../../ROADMAP-CORE.md) | shell, FS, tasks, drivers, kernel services |
| `gui` | [`ROADMAP-GUI.md`](../../../ROADMAP-GUI.md) | framebuffer, fonts, mouse, windowing, on-screen UI |

An automation (see "Setting up the Automations" below) runs this procedure
for **one** track per run. CI (`.github/workflows/ci.yml`) auto-merges the
result if it passes. Follow these steps in order.

## 0. Determine your track

Resolve `TRACK` from the automation prompt or user message:

- Explicit `TRACK=core` / `TRACK=gui` (preferred).
- Phrases like "core roadmap" / "terminal track" → `core`.
- Phrases like "GUI roadmap" / "graphics track" → `gui`.

If the track is still ambiguous, **stop** and say you need `TRACK=core` or
`TRACK=gui` - do not guess, and do not pick from both files.

Set:

- `TRACK=core` → task file `ROADMAP-CORE.md`
- `TRACK=gui` → task file `ROADMAP-GUI.md`

## 1. Pick exactly one task

Open the task file for your track and find the **first** unchecked task
(a line starting with `- [ ]`), in file order.

**Dependency skip:** if that task (or its bullet block) says
`Depends on: ...` and the named dependency is not yet done (still `- [ ]`
or missing), skip it and take the next unchecked task in the **same**
file that is unblocked. Never implement a task from the other track's
file in this run.

Do **one** task per run. Small, reviewable, independently-mergeable PRs
are the entire point - resist bundling multiple tasks "while you're in
there."

If there are no unchecked, unblocked tasks left in your track file, stop
and say so (don't invent new work, don't poach the other track).

## 2. Read the relevant skill

Each phase links to a skill under `.cursor/skills/` (e.g.
`interrupts-and-exceptions`, `memory-management`, `graphics-and-gui`).
Read it before writing any code - it has conventions, gotchas, and
lessons from past mistakes.

## 3. Implement the task

- Create a branch following this repo's `cursor/<descriptive-name>`
  convention (the exact suffix is assigned automatically per agent run -
  don't hardcode one from a past session).
- Implement *only* what the task describes. If it needs to be split further
  once you're in the code, do the smallest coherent slice and leave the
  rest as a new task in the **same** track file.
- Respect soft ownership from the track file. Shared files (`main.rs`,
  `boot/boot.asm`, `input.rs`) get minimal, API-shaped diffs.
- Follow existing conventions (driver shape in `kernel/src/vga.rs` /
  `serial.rs`, defensive/non-panicking error handling, doc comments on
  `unsafe fn`, etc.) - see `.cursor/skills/drivers/SKILL.md` and
  `.cursor/skills/qemu-testing-and-verification/SKILL.md`.

## 4. Verify against the task's "Done when" criteria

At minimum:

```bash
cd kernel && cargo clippy -- -D warnings
cd .. && make test
```

`make test` runs `scripts/ci-test.sh` - the same headless boot check CI
runs. If the task's "Done when" criteria describe something `make test`
can't check (a visual framebuffer change, keyboard/mouse input, a
deliberate exception, etc.), also do the manual/visual verification in
`.cursor/skills/qemu-testing-and-verification/` (a real QEMU screendump).

**GUI track:** also rebuild and spot-check the browser GUI page:

```bash
./scripts/build-web-demo.sh
# serve _site/ and open /gui.html - scaled canvas + serial log
```

See `.cursor/skills/web-demo-packaging/`. Boot-time BIOS, disk I/O, and
video mode changes have diverged under v86 before.

Do not open a PR for something you haven't actually verified boots
correctly - CI catches a totally broken boot, but not "VGA/framebuffer
output is garbled."

## 5. Update the roadmap (and README, if needed)

- Check off the task in **your track file only** (`- [ ]` → `- [x]`) in
  the same PR. Do not edit the other track's checkboxes unless you
  accidentally duplicated a dependency note that must stay accurate.
- If that was the **last** task of a milestone called out in the README
  `## Status` list, check off the matching README item too.

## 6. Commit, push, and open a PR

- Commit message: reference the track and task, e.g.
  `Core 6.1: add shell ls built-in` or `GUI 5.2: framebuffer pixel primitives`.
- Push the branch and open a PR against `main`.
- PR description: state `TRACK`, which task, link to the track file
  entry, and how it was verified (step 4).
- **A draft PR is fine.** The Cursor Automation's PR tool has no draft
  option and creates drafts, so `ci.yml`'s `ready-for-review` job marks
  `cursor/*` PRs in this repo ready automatically. Don't try to flip it by
  hand - just don't count on the PR still being a draft a minute later.
  That job depends on the repo's `PR_READY_TOKEN` secret (the default
  `GITHUB_TOKEN` cannot take a PR out of draft); if the secret is missing,
  the PR is tested but stays a draft - say so in your final message
  rather than working around it.

From here, `.github/workflows/ci.yml` takes over: it takes the PR out of
draft, builds the kernel, runs `cargo clippy`, runs the boot test, waits
for and respects the `Cursor Bugbot` check if Bugbot is enabled (see
`.cursor/skills/bugbot-and-code-review/SKILL.md`), and - only for
`cursor/*` branches in this repository, and only if all of that passes -
merges the PR automatically (squash + delete branch). If CI fails, or
Bugbot flags something, the PR is left open for the next run (or a human)
to fix; **never bypass a failing check to force a merge.** If you're
resumed on a PR blocked specifically by a Bugbot finding, follow
`.cursor/skills/bugbot-and-code-review/SKILL.md` rather than this
procedure - fix exactly what Bugbot flagged, nothing else, and push to the
same branch.

## Setting up the Automations themselves

This skill describes what an agent run should *do*; it doesn't create the
schedule/trigger - that's a
[Cursor Automation](https://cursor.com/automations), configured through the
Cursor dashboard (or the `/automate` skill in a local chat session).

Create **two** automations for this repo (same triggers, different
prompts):

### Core automation

- **Repository**: this one (`GOATos`).
- **Prompt**:
  > Follow the procedure in `.cursor/skills/roadmap-automation/SKILL.md`
  > with `TRACK=core`. Pick up and complete the next unchecked, unblocked
  > task in `ROADMAP-CORE.md` only.
- **Trigger**: `Pull request merged` on `main`, optionally plus a daily
  schedule as a fallback if the chain stalls.

### GUI automation

- **Repository**: this one (`GOATos`).
- **Prompt**:
  > Follow the procedure in `.cursor/skills/roadmap-automation/SKILL.md`
  > with `TRACK=gui`. Pick up and complete the next unchecked, unblocked
  > task in `ROADMAP-GUI.md` only. Verify graphics changes via QEMU
  > screendump and the Pages GUI demo (`gui.html`).
- **Trigger**: same pattern as core (`Pull request merged` on `main`,
  optional daily schedule).

Make sure "Pull request creation" is enabled for both (default). Whether
they create drafts doesn't matter - `ci.yml` marks `cursor/*` PRs ready
itself, per step 6 above.

If both automations fire on the same merge, that's expected: one core PR
and one GUI PR may open in parallel. Soft ownership + small diffs keep
conflicts rare; CI serializes merges.
