# Office Hub: Feishu

Open **Services → Office** for Feishu documents, calendars, tasks and Base.
The page has **Documents** and **Calendar & tasks** tabs and opens on Documents.
The Tauri desktop shell and desktop-class WebUI are supported. All business
operations use the official Feishu CLI. Components and credentials belong to
the backend machine, which may differ from the browser's computer.

## Connect Feishu

The page shows the next action for its current connection state:

- **Component missing**: the main page explains installation and provides an
  install action. Downloading begins only after you choose it. Installation has
  progress and retry states. Compatible installations are reused; broken or
  incompatible installations offer repair guidance.
- **Application not connected**: select an existing connection or follow the
  official Feishu application setup flow.
- **Signed out or expired**: complete Feishu sign-in in your browser, then return
  to see the current account.

Basic sign-in requests only `offline_access`. Document, calendar and task access
is requested when you use those features. There is no mandatory permission
checklist after sign-in.

## Accounts and switching users

Click the current account in the header to open account information and actions:

- **Change account**: confirm sign-out, then open a fresh Feishu sign-in flow and
  choose the account to use.
- **Sign out**: confirm sign-out from this connection without starting another
  sign-in automatically.
- **Switch connection**: choose another configured Feishu application connection.

Sign-out retains application setup and submitted request results. Signing in
again does not reinstall the component. Wait for active saves or authorization
cleanup before signing out. The official CLI clears every user stored in the
selected connection, so connections containing multiple users are not signed out
automatically. Use **Switch connection** instead to avoid signing out other users.

The permissions area shows granted and missing document, calendar and task
access, without selecting permissions by default. Granting access returns to the
current input; Save still requires your action. Changing the application, user or
profile prevents the old form from writing. Cancelling authorization waits for
the original process to settle before another flow can begin.

Advanced settings contain the component host, version, path, read-only update
check and connection disable action. **Disable connection** stops new business
work while retaining sign-in; it is distinct from signing out. The verified
baseline is 1.0.98. Checks do not install or synchronize skills; external
installations retain their original maintenance method. The full official setup
command is provided for you to copy into a terminal. It may install global skills
for other agents and is never run silently. Secrets and tokens remain in the
CLI's own store.

## Documents

Browse accessible documents, search by title and filter by document type. Queries
and filters apply to the server collection before pagination. Click a document
name or **Open in browser**, or paste an official Feishu/Lark resource link, to
open it in the browser. Document bodies are read and edited in Feishu.

Create a document by title; the blank document appears in the list and its body
can be written in Feishu. Delete first shows the exact title and link for
confirmation. Only supported cloud document types offer deletion; Wiki nodes and
Base resources remain browser entries. An unresolved deletion blocks another
write to the same resource and keeps access to its original result.

## Calendars and tasks

Personal events use explicit times and a timezone, without automatically adding
participants or a video meeting. Tasks have a title, real assignee and optional
due date. Creator, assignee and list are separate. **Assigned to me** and
**Created by me** are different lists; choosing a shared list grants no new member
permissions. Completing or reopening a task also needs write access.

## Base

Choose the **Base** filter in Documents, search for the resource and open it in
the browser. Base has no separate page or record editor in NomiFun.

## Results

Save submits directly from the form and shows its result there. Unresolved
operations remain accessible from the original resource for verification.

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

Already-started remote work may complete even after disabling the connection.
Disabling is distinct from signing out or revoking Feishu permissions.

The opt-in [Feishu Skill](mcp-and-skills.md) continues through canonical process
tools. Missing components use the same Office supplier; the Office page's path
and profile selections are not automatically injected into Agent sessions.
