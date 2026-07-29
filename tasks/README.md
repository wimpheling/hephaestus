# Coding-agent task system

This directory is the repository-local work queue for coding agents. Task
state is represented by the task file's location:

| Directory | Meaning |
| --- | --- |
| `tasks/todo/` | Planned work that has not started. |
| `tasks/in-progress/` | Work currently owned and being implemented. |
| `tasks/done/` | Completed and verified work retained as project history. |

## Workflow

1. Select one task from `tasks/todo/`.
2. Move the file, without copying it, into `tasks/in-progress/` before making
   implementation changes.
3. Record the active agent or session in the task's `Owner` field.
4. Check an item only after its implementation and relevant verification are
   complete.
5. Keep newly discovered work in the same task when it is necessary for the
   stated acceptance criteria. Create a separate todo task when it is
   independently deliverable or outside the current scope.
6. Do not move a task to `tasks/done/` while required checkboxes remain open.
   Split genuinely deferred work into a new todo task and link it from the
   completed task.
7. Move the fully checked task to `tasks/done/` only after formatting, linting,
   tests, documentation checks, and task-specific acceptance tests pass.

The directory is the authoritative status. Do not add a competing status field
whose value can disagree with the path. An `Owner` identifies coordination
responsibility, not task state.

## Task-authoring rules

Use lowercase kebab-case filenames that describe the outcome, for example
`reusable-agent-releases-and-instances.md`.

Architectural context, locked decisions, dependencies, and non-goals may be
plain prose or tables. Every implementable workstream, subsection, requirement,
test, documentation change, and verification command must be represented by a
Markdown task-list checkbox. Tasks must be detailed enough that another coding
agent can continue after interruption without reconstructing the plan from
chat history.

Check boxes represent verified outcomes:

- An unchecked item is not complete.
- A checked implementation item has code and focused tests where applicable.
- A checked verification item records a command that passed in the current
  worktree.
- Partially implemented items remain unchecked and may include a short progress
  note beneath them.

## Task template

```markdown
# Task title

Owner: unassigned

## Outcome

Describe the user-visible and architectural outcome.

## Locked decisions

Use a table for decisions that implementations must preserve.

## Non-goals

Use prose or a table for explicitly excluded work.

## Implementation checklist

- [ ] **Workstream**
  - [ ] **Implementable subsection**
    - [ ] Implement one independently verifiable requirement.
    - [ ] Add focused tests for the requirement.

- [ ] **Verification**
  - [ ] Run the repository-specific formatting command.
  - [ ] Run the repository-specific lint command.
  - [ ] Run the required test suites.
  - [ ] Run documentation checks.

## Completion evidence

Record commands, test counts, relevant artifact IDs, and deliberate follow-up
tasks before moving the file to `tasks/done/`.
```
