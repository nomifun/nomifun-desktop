---
name: planning-with-files
description: Maintain durable task plans, findings, and progress logs in workspace files for long-running or research-heavy work. Use when a task spans many steps or sessions, needs an auditable evidence trail, or risks losing important decisions to context limits. Do not use for short tasks that fit comfortably in the active conversation.
---

# Planning with Files

Use workspace files as durable working memory for complex tasks. Keep the files concise, current, and useful to another agent resuming the work.

## Decide Whether to Use This Skill

Use it when at least one condition applies:

- The task has several dependent phases and will take sustained work.
- Research findings, commands, links, or decisions need an audit trail.
- The work may continue in another session or be handed to another agent.
- The amount of evidence is likely to outgrow the active context.

Skip it for simple edits, short explanations, or tasks that can be completed safely without persistent notes.

## Files

Create only the files the task needs, normally in the workspace root:

- `task_plan.md`: goals, scope, milestones, decisions, and current status.
- `findings.md`: verified facts, evidence, source paths, constraints, and open questions.
- `progress.md`: chronological work log, validation results, failures, and handoff notes.

Before creating anything, search for existing files with these names. Resume relevant files instead of overwriting them. If existing files belong to another task, use a clearly named subdirectory or task-specific filenames.

When several people or agents share the workspace, designate one planning-file writer at a time. Handoff ownership explicitly before another writer updates the files.

## Workflow

1. Inspect the workspace and read any existing planning files.
2. Write a small plan with one active milestone and concrete completion criteria.
3. Record evidence in `findings.md` as it is verified; distinguish facts from hypotheses.
4. Update `progress.md` after meaningful changes, tests, or blockers.
5. Re-read the plan before major decisions and revise it when evidence changes the approach.
6. At completion, mark every milestone accurately and leave a concise final handoff with validation results.

## `task_plan.md` Shape

Keep the plan operational:

```markdown
# Objective

One sentence describing the desired outcome.

## Scope

- In scope: ...
- Out of scope: ...

## Milestones

- [ ] In progress: current concrete step
- [ ] Pending: next concrete step
- [x] Completed: verified result

## Decisions

- Decision — reason and evidence.

## Completion criteria

- Exact checks that must pass before the task is done.
```

Maintain at most one in-progress milestone. Do not mark work complete when required validation is still failing.

## Evidence and Progress Rules

- Store large raw output in its natural artifact or log; summarize it in the planning files.
- Record exact paths, commands, test names, and error messages when they matter for resuming work.
- Sanitize command output and error text before recording it; preserve the useful failure signature without copying credentials or private data.
- Add dates only when sequencing or freshness matters.
- Never store secrets, access tokens, private keys, or unnecessary personal data.
- Keep the current conclusion in `findings.md`; record reversals and their evidence chronologically in `progress.md` so the audit trail remains intact.
- Keep planning files about the task, not a transcript of every action.
- Do not stage, commit, ignore, move, or delete planning files unless the user or repository policy calls for it.

## Completion

Before handing off, ensure the plan reflects the real state, the findings contain the decisive evidence, and the progress log names the final verification performed. Do not delete or archive the files unless the user requests it.
