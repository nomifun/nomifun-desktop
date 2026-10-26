# Command discovery and common workflows

Every `lark-cli` example uses the absolute launcher selected by
[shared executable discovery](installation.md), including help commands.
Start with that installed CLI's help for the relevant service and shortcut.
Prefer a `+shortcut` when it covers the operation. For generated API commands,
inspect `schema` before constructing JSON. Use `api` only when the higher
layers do not cover a verified endpoint; it retains the same identity,
permissions and authorization requirements.

```text
lark-cli --help
lark-cli docs --help
lark-cli docs +fetch --help
lark-cli schema calendar.events.instance_view
```

Use the actual command help to distinguish a JSON input flag from JSON output
formatting. Keep payloads as single literal arguments, or use stdin when the
command supports it. Do not paste Bash quoting, pipes or variable assignments
into a process argument array. For file flags, use task-owned relative paths
under `cwd`; check command-specific path support before using other paths.

## Choose a service

| User request | Help entry point | Workflow |
| --- | --- | --- |
| Read, create or edit cloud documents | `docs --help` | Use the document URL/token; fetch before editing existing content. |
| Find/upload/download files, comments or sharing | `drive --help` | Resolve the existing file and intended folder; retain file IDs. |
| Base / bitable records, fields or views | `base --help` | Resolve the Base link, table and field schema before changing records. |
| Cloud spreadsheet cells and ranges | `sheets --help` | Read sheet metadata and the target range before updating cells. |
| Cloud presentations | `slides --help` | Discover the presentation and slide IDs before editing. |
| Messages and group chats | `im --help` | Resolve the chat/recipient and inspect the exact send/reply/search command. |
| Agenda, meetings and invitations | `calendar --help` | Use an explicit timezone/range; verify participants before creating an event. |
| Tasks and task lists | `task --help` | Resolve the task/list ID; retain ownership and due-date semantics. |
| Email | `mail --help` | Separate search/read from a user-requested send, reply or forward. |
| Wiki spaces and nodes | `wiki --help` | Distinguish a wiki node ID from the underlying document token. |
| Find people | `contact --help` | Resolve the actual user ID; do not infer it from a display name. |

Other services are discoverable through root help. Feishu Project uses the
separate upstream `meegle-cli`; it is not installed by this skill. Local
`.docx`, `.xlsx` and `.pptx` editing belongs to the `officecli` skills.

## Read a linked document

After inspecting help, replace the placeholder with the user's actual URL:

```text
lark-cli docs +fetch --doc <document-url-or-token> --as user
```

For long documents, use the supported outline/section/keyword options to
retrieve the relevant part. Keep the JSON response's content and reference
metadata together. An excerpt is not the whole document. For edits, fetch the
IDs and structure required by `docs +update --help`, apply the requested
change, and verify the changed part. Copying an existing document should use
the file-copy operation rather than recreating its body.

## Read an agenda

```text
lark-cli calendar +agenda --help
lark-cli calendar +agenda --as user
```

Inspect the defaults before interpreting a relative request such as “next
week.” Pass the supported date/time flags for that exact period. An empty
result only covers the queried identity, calendars and range.

## Send a requested message

Resolve the destination from `im`/`contact` search results, not a guessed ID.
Inspect `im +messages-send --help`. When supported, inspect the exact payload with `--dry-run`:

```text
lark-cli im +messages-send --chat-id <resolved-chat-id> --text <requested-text> --as user --dry-run
```

Once the recipient/content are authorized, run that same argument list
without `--dry-run`. If supported, generate an idempotency key before the
first send and retain it for retries of that identical operation. If delivery
is uncertain, reconcile using the returned message ID or chat history before
sending again. A missing history result alone does not prove non-delivery:
without a conclusive result or a supported idempotent retry, report the
uncertainty and do not automatically resend. Return the message link/ID from
the actual response when available.

## Base and spreadsheet updates

For Base links, inspect `base +url-resolve --help` and resolve the link before
choosing a table. Read the target field schema and records, then construct
only the changed fields using the installed shortcut's input schema. Do not
treat a displayed primary-field value as a record ID. For Sheets, preserve
unaffected cells and formulas by targeting the requested range.

## Pagination and verification

Keep searches scoped to the task. When supported, `--page-all` with
`--page-limit` bounds automatic pagination; some shortcuts instead use an
offset or cursor. Follow the actual help and returned pagination fields.
If `has_more` or a cursor remains, state that the results are partial.

Inspect warnings and per-item outcomes even with `ok: true`. For asynchronous
operations, use the returned task/status command before claiming completion.
A dry run is not an applied change. For writes with an uncertain outcome,
read the existing state rather than automatically replaying the request.

Detailed upstream examples are available in the
[official service skills](https://github.com/larksuite/cli/tree/main/skills).
Consult only the relevant service reference when help is insufficient; these
upstream files are not additional installed NomiFun skills or tool grants.
