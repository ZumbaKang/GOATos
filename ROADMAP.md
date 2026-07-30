# GOATos Roadmap

The README's [Status](README.md#status) list is intentionally short. Detailed
tasks live in **two parallel tracks** so the graphical UI can improve on its
own while core / terminal features keep landing for the GUI to consume later.

| Track | File | What it owns |
|-------|------|----------------|
| **Core** | [`ROADMAP-CORE.md`](ROADMAP-CORE.md) | Interrupts, memory, drivers, shell, FS, scheduling, kernel services |
| **GUI** | [`ROADMAP-GUI.md`](ROADMAP-GUI.md) | Framebuffer, fonts, mouse, windowing, on-screen UI |

**Shared kernel, separate queues.** Both tracks ship into the same
`kernel/` tree and the same `build/disk.img`. What is separate is *which
unchecked task an automation picks*, plus soft file ownership (see each
track file). CI (`.github/workflows/ci.yml`) stays shared: every PR must
still boot and pass `make test`.

## How automations use this

Two [Cursor Automations](https://cursor.com/automations) should run the
procedure in
[`.cursor/skills/roadmap-automation/`](.cursor/skills/roadmap-automation/SKILL.md)
with different tracks:

- **Core automation** prompt: follow that skill with `TRACK=core` (tasks
  from `ROADMAP-CORE.md` only).
- **GUI automation** prompt: follow that skill with `TRACK=gui` (tasks
  from `ROADMAP-GUI.md` only).

Each run still does **exactly one** task, opens one PR, and lets CI
auto-merge `cursor/*` branches when checks pass. Humans can do the same
manually anytime.

### Skipping blocked tasks

If the first unchecked task in a track says `Depends on: ...` and that
dependency is not checked off yet, skip to the next unchecked task in the
**same** track that is unblocked. Never steal a task from the other track's
file.

### Merge conflicts

If both tracks must touch a shared file (`main.rs`, `boot/boot.asm`,
`input.rs`), keep the diff minimal and API-shaped (call a new function,
don't reformat the whole bring-up). Prefer adding a core API in one PR and
consuming it from GUI in a later PR over editing both sides at once.

## Browser demos

The same disk image is published to GitHub Pages:

- **Hub:** `web/index.html` → site root
- **GUI / framebuffer view:** `web/gui.html` (scaled Mode 13h canvas +
  serial log) - this is the page to open when testing graphics work
- Build locally: `./scripts/build-web-demo.sh` then
  `python3 -m http.server -d _site 8080`

## Historical note

Phases 0-4 were completed on a single linear roadmap and now live under
`ROADMAP-CORE.md`. Phase 5 (graphics) moved to `ROADMAP-GUI.md` and can
advance in parallel with Core Phases 6+.

