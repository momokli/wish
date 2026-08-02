# Development Workflow

> The pattern we use for iterative, agent-assisted development on this project.

---

## The Cycle

```
┌──────────────────────────────────────────────────────────────┐
│                     PLANNING PHASE                           │
│                                                              │
│  Orchestrator (you) spawns REVIEWER agent                    │
│                                                              │
│  Reviewer reads:                                             │
│    • notes.md (consolidated findings from last cycle)        │
│    • native_deemix_control.md (target + work log)            │
│                                                              │
│  Reviewer writes into native_deemix_control.md:              │
│    • Work log entry (retrospective on last execution phase)  │
│    • Updated "Current understanding" if we learned something │
│    • Next step section: 1–5 concrete sub-agent tasks         │
│      in a table format the orchestrator can parse            │
│    • Flagged issues to "Open questions"                      │
│                                                              │
│  Orchestrator reads native_deemix_control.md,                │
│  parses the task table, spawns sub-agents                    │
│                                                              │
├──────────────────────────────────────────────────────────────┤
│                     EXECUTION PHASE                          │
│                                                              │
│  Sub-agents work in parallel:                                │
│    • Each writes findings to notes_<label>.md                │
│    • NEVER writes to notes.md directly                       │
│    • Reports back with brief summary only                    │
│                                                              │
│  Orchestrator spawns COORDINATOR agent                       │
│                                                              │
│  Coordinator:                                                │
│    • Reads all notes_*.md files                              │
│    • Merges into notes.md (appends under dated heading)      │
│    • Deletes notes_*.md files after merge                    │
│    • Flags contradictions between sub-agents                 │
│                                                              │
├──────────────────────────────────────────────────────────────┤
│                        CHECK                                 │
│                                                              │
│  Orchestrator spawns REVIEWER agent (same role, new cycle)   │
│                                                              │
│  → Back to PLANNING PHASE                                    │
└──────────────────────────────────────────────────────────────┘
```

---

## Roles

### Orchestrator (you — the top-level agent)

- Spawns REVIEWER, SUB-AGENTS, COORDINATOR in sequence
- Reads `native_deemix_control.md` after reviewer runs to parse the task table
- Spawns sub-agents based on that table
- Only writes code when there's no sub-agent to delegate to

### Reviewer

- **Reads**: `notes.md` + `native_deemix_control.md`
- **Writes**: `native_deemix_control.md` (work log, understanding, next tasks, open questions)
- **Never executes code**
- **Task table format** — writes next tasks as a markdown table so the orchestrator can parse it:

  ```
  | Label | Task | Writes to |
  |-------|------|-----------|
  | setup | Fork deemix, add packages/musdl | notes_setup.md |
  | deps  | Verify dependencies compile    | notes_deps.md |
  | smoke | Minimal download script        | notes_smoke.md |
  ```

### Sub-agent

- Gets one row from the reviewer's task table
- **Writes findings to** `notes_<label>.md` — NEVER to `notes.md`
- Returns brief summary to orchestrator
- Does NOT plan next steps

### Coordinator

- **Reads**: all `notes_*.md` files present in the project
- **Writes**: `notes.md` — appends findings under a dated heading
- **Deletes**: `notes_*.md` files after merging
- Handles missing files gracefully (agent may have failed)
- Flags contradictions: "Agent A found X, Agent B found Y — these disagree"
- Does NOT plan next steps, does NOT execute code

---

## File conventions

| File                       | Owner       | Purpose                                                                         |
| -------------------------- | ----------- | ------------------------------------------------------------------------------- |
| `native_deemix_control.md` | Reviewer    | Target, work log, current understanding, next step (task table), open questions |
| `notes.md`                 | Coordinator | Consolidated findings, append-only with dated headings                          |
| `notes_<label>.md`         | Sub-agent   | Per-agent scratchpad (temporary — deleted by coordinator)                       |
| `workflow.md`              | Human       | This file — the process definition                                              |

### `notes.md` format

Coordinator appends under dated headings:

```markdown
## 2026-07-24 — Cycle 1

### Agent A (label)

Findings...

### Agent B (label)

Findings...

### Contradictions / flags

- Agent A says X, Agent B says Y
```

---

## Rules

1. **Sub-agents never write to `notes.md`.** Only `notes_<label>.md`.
2. **Coordinator never plans.** Only merges + flags contradictions.
3. **Reviewer never executes.** Only reads, checks, and proposes tasks.
4. **Every cycle**: plan → execute → merge → check → plan → ...
5. **Task table is the contract** between reviewer and orchestrator. If the orchestrator can't parse it, the cycle breaks.
6. **If stuck, say so.** Reviewer flags it in "Open questions". Coordinator flags contradictions. No pretending.
7. **Small tasks.** Each sub-agent task completable in one turn. 1–5 tasks per cycle (not always exactly 3–5).
8. **Missing files are OK.** Coordinator handles absent `notes_*.md` — some agents may fail. Don't block the cycle.
