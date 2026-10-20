# Scheduled tasks

Scheduled tasks submit work to canonical AgentSessions while NomiFun is running and the computer is awake. They use the same Agent, model, resource and runtime contracts as ordinary conversations.

For one-off background work that should start promptly, see [AutoWork](./autowork-requirements.md).

## Create a task

Open `/scheduled` and choose New task. Set its name, instructions and schedule.

- **Agent:** select an official Agent or personal preset from the Agent workbench. The default is General (`assistant.general`). CLI/backend selection has been retired.
- **Model:** independently select a configured Chat provider/model. Editing preserves an explicit saved selection; changing Agent does not clear it. An unavailable or incompatible model produces an explanation rather than a silent replacement.
- **Project:** optionally select an existing directory. The host supplies a managed workspace otherwise. A directory path is not an authorization grant.
- Official Agents can be prepared using the chosen model without first saving an official author default. Personal presets need an executable saved configuration.

The host resolves the Agent, model, extensions and resource grants when saving the task, then freezes exact Revision/Snapshot references and typed resource bindings. A task-specific model does not change workbench defaults or the installation model preference.

Renaming, changing instructions or changing the clock schedule preserves the frozen execution configuration. Explicit Agent/model/project changes use the normal host preparation contract.

## Session modes

| Mode | Behavior |
| --- | --- |
| New conversation | Each occurrence creates a distinct Session from the frozen task binding, subject to existing concurrency settings. |
| Continuing conversation | The first valid opening fixes one Session before the first Turn. Later failures, waiting or cancellation do not silently create another continuing workspace. |
| Specified conversation | Reuse the selected Session and its canonical Agent, model and project. |

A bound task does not keep a shadow execution configuration. Change its Agent/model through the Session's explicit configuration transition.

The existing clear-context-before-each-run option remains available for continuing Sessions. The same Session owner clears context without deleting history or rebuilding the Session; active/paused Turns and unsettled effects remain protected.

The task text is submitted unchanged. Background execution rules use the separate system instruction lane, so host-added numbering cannot become a fabricated user task plan. Real numbered steps in the task retain their normal ordering protections.

Overlapping work targeting the same Session is recorded as `skipped`; it does not cancel the active Turn or replay its input. Stop this run and disable future triggers remain distinct operations.

## Scheduling and recovery

Schedules support one-off `At`, fixed-interval `Every`, and cron expressions with IANA timezones. Five-field Unix expressions receive an implicit zero seconds field; six-field expressions can specify seconds.

```text
minute hour day month weekday
0 9 * * MON
```

This runs at 09:00 every Monday in the task's timezone. The form offers common frequencies and validates custom expressions.

A scheduled occurrence is reserved in the same database transaction that verifies owner/revision/planned time and advances its successor. Timers install committed schedule times. Finishing an Agent run does not advance the schedule again. Long-running new-session work can coexist under existing concurrency settings, while continuing-session overlaps produce explicit skips.

Startup/wake retains the established missed-run policy: record `missed` and install a future schedule rather than replaying every missed interval. Previously reserved runs reconcile their original exact Session/Turn receipts; recovery cannot resend their inputs.

## Results and control

Task details show run history, original Session links and classified failures. History projects terminal reservations directly; a presentation limit never prunes the idempotency, cancellation or recovery evidence.

| Status | Meaning |
| --- | --- |
| `ok` | The exact Turn confirmed successful public delivery. |
| `error` | Preparation or execution failed, with application/model/tool ownership explained. |
| `skipped` | The occurrence did not run, for example because its Session was busy. |
| `missed` | The host was unavailable/asleep at the scheduled time. |
| `input_required` | The Agent handed control back for user input. |
| `cancelled` | The exact run was cancelled by its owner. |

Missing local workspace/process grants are a local Agent preparation failure, before model dispatch. They do not become an upstream-provider error or an ineffective retry suggestion. Provider authentication, billing, rate limiting, network and protocol failures retain their established typed diagnoses. Unknown ownership stays unknown.

Failures are read from the original canonical terminal event. A preparation rejection before any Turn belongs to the original reservation. Historical failures remain unchanged, and unknown effects never authorize automatic replay.

Keep-awake only inhibits system sleep; it does not replace a running host or recovery. Completed new-conversation runs retire their owned heavy resources after actual settlement evidence, while continuing workspaces and legitimate background responsibilities remain protected.

## Schedule from a conversation

Workbench Agents use the native `automation.schedule` actions for the current Session. The host derives owner and Session from the authorized invocation; tool parameters cannot impersonate another owner or replace arbitrary provider/model/Session identities. The retired text-command protocol is no longer injected.

## Task skills

Per-task skills remain supported. SQLite `skill_content` is authoritative text; `cron/skills/<job-id>/SKILL.md` is a generated artifact, never a fallback configuration source. Deleting a task removes its generated skill directory; produced Sessions retain their own history and lifecycle.

## Configuration upgrades

Verifiable current-generation frozen bindings are normalized without changing their artifact references. Legacy CLI tasks and configurations without provable complete bindings retain their names, schedules, instructions and chosen models, but their old execution ingress is disabled with a reconfiguration explanation. Saving a workbench configuration enables them again. Upgrades do not silently grant the broader General Agent permissions.

Fault tests use isolated data. Supported-schema corruption and I/O failure still fail closed; incompatible database lineages remain under the existing dataset coordinator, without deleting user databases by hand or rewriting committed migration checksums.

## API

| Operation | Route |
| --- | --- |
| List/create | `GET/POST /api/cron/jobs` |
| Read/update/delete | `GET/PUT/DELETE /api/cron/jobs/:id` |
| Run now | `POST /api/cron/jobs/:id/run`, with a stable `Idempotency-Key` |
| Runs/Sessions | `GET /api/cron/jobs/:id/runs`, `.../:id/conversations` |
| Task skill | `GET/POST/DELETE /api/cron/jobs/:id/skill` |

Create requests contain workbench `agent_selection`, an independent `model`, `message`, and any explicit workspace/resource choices. Only the host authors frozen bindings. CLI/backend fields and client-authored snapshots are rejected. Specified-session tasks submit a Session reference rather than copied execution configuration.
