# Office Hub: Feishu

Open **Common → Office** for Feishu documents, calendars, tasks and Base.
The Tauri desktop shell and desktop-class WebUI are supported. All business
operations use the official Feishu CLI. Components and credentials belong to
the backend machine, which may differ from the browser's computer.

## Connect Feishu

The connection dialog has three steps:

1. **Prepare environment**: check the backend host and component. Reuse a compatible
   installation; prepare a missing component only after an explicit click. The CLI
   is not included in the NomiFun installer.
2. **Connect Feishu**: follow the official application setup or sign-in link.
   Existing profiles, explicit executable paths and manual application details
   are available in advanced settings.
3. **Read permissions**: choose the documents, calendars, tasks or Base data you
   need to browse, or grant access later. An administrator may need to approve
   access. Write permissions are requested when you actually save.

Step two signs in with the CLI's explicit `offline_access` scope, without
requesting business permissions. You can skip step three and grant access when
you first use the corresponding feature.

Granting access returns to the current input; Save still requires your action.
Changing the application, user or profile prevents the old form from writing.
Cancelling authorization waits for the original process to settle before another
flow can begin.

Advanced settings provide a read-only update check, automatic discovery and the
full official setup command to copy into your own terminal. The verified baseline
is 1.0.98. Checks do not install or synchronize skills; external installations
retain their original maintenance method. The full wizard may install global
skills for other agents and is never run silently. Secrets and tokens remain in
the CLI's own store.

## Documents

Search or paste a link to an accessible document. New documents require a title;
body content is optional. Recent entries retain only an actually opened resource's
locator, title and time. Opening one reads its current content from Feishu again.

Select one editable ordinary paragraph inside a document, change its text and
click **Save**. The workbench uses the real block identity and original revision.
Headings, complex formatting and tables remain read-only; open Feishu to edit them.
Without a reliable block/revision baseline, the document remains readable. A first
paragraph can be inserted only into a body verified to be empty. If the source
revision changes, reload it while retaining your current input.

An unresolved save protects the whole document from another write. Selecting a
different paragraph does not bypass that protection.

## Calendars and tasks

Personal events use explicit times and a timezone, without automatically adding
participants or a video meeting. Tasks have a title, real assignee and optional
due date. Creator, assignee and list are separate. **Assigned to me** and
**Created by me** are different lists; choosing a shared list grants no new member
permissions. Completing or reopening a task also needs write access.

## Base

Paste a complete Feishu link containing `/base/` and the `table` parameter. If it
is incomplete, select the table in Feishu and copy its link again. Fields, options,
record identities and pagination come from actual responses.

Select records, enter supported simple field values and click **Save** once,
for at most 200 records in one batch. System, formula and complex fields are not
ordinary writable values. Original values are checked before writing, but this
does not prevent simultaneous changes by another client.

An accepted batch initially remains **Pending verification**. Confirmation reads
back the original records and fields. Values not yet visible are not proof of
non-execution. Pending rows cannot be selected for another write; open their
original operation to check again.

## Results and disabling

Save submits directly from the form. Activity includes work initiated by this
workbench only.

- **Confirmed** retains real resource identities and original receipts.
- **Proven not executed** allows corrected items to be submitted explicitly as
  a new request, while retaining the previous record.
- **Pending verification** may already have executed. Check the original request
  or target; it is never resent automatically.
- **Partially complete** shows each outcome. Successful and unknown items cannot
  be included in a new submission.

A lost response is recovered by its original request ID. Reloading, navigating
away or restarting does not replay writes. Without a task GUID or exact receipt,
same-name search cannot establish creation. Original outcomes and later checks
remain distinct.

Disabling stops new business work and retains the connection, recent entries and
history. Already-started remote work may complete. It does not revoke Feishu
permissions; manage revocation in Feishu.

The opt-in [Feishu Skill](mcp-and-skills.md) continues through canonical process
tools. Missing components use the same workbench supplier; the workbench's path
and profile selections are not automatically injected into Agent sessions.
