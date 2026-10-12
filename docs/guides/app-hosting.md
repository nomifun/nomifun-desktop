# NomiFun applications

The **Apps** home page has **Create** and **Import** entries. Cards show names,
descriptions, tags and actual runtime state. Search matches names, descriptions
and tags; filters and pagination use current registered applications. Open a
card to return to that application's workspace.

## Create, develop and edit

1. Choose **Create**. The left pane contains an ordinary conversation; the right
   pane contains the current application, settings, operations and preview.
2. The composer uses the fixed **Built-in application** Agent. Select a usable
   model and describe the application. Its owned directory is already the
   canonical Session workspace; no separate Agent or folder setup is needed.
3. The Agent uses ordinary file and process tools, reads guidance on demand, and
   configures the same application's name, description, tags and commands. The
   complete messages, reasoning, tools and history remain an ordinary chat.
4. Inspect settings and real output, preview, or start using the application.
   A running process does not prove that its business functions work.
5. Reopen the card and choose a retained **Project session**, or choose **New
   session** for the next change. Sessions share the same application directory;
   edits target current files without another import. Rebuild and start when needed.

Each application corresponds to one independent project with its own managed
directory. Different applications do not use the same project; importing the same
external source into two applications creates two independent copies. A project's
ordinary conversation workspace can contain multiple sessions. Its history
shows names, times and runtime state, including archived sessions. Files and settings
belong to the project; switching sessions preserves the right-hand preview and unsaved
settings. The unified canonical Session store keeps conversations, without copying
another chat log into the source directory.

A new session starts from current project files with its own chat context. Within
the current Host runtime, project memory written by the Agent can be read by new
sessions in the same project. This memory is not persisted across Host restarts. New
sessions in different projects use separate memory scopes. Existing sessions retain
their frozen configuration and prior memory scope; a new session does not load all
project messages automatically. Ordinary conversations using the same directory
also appear in the project history. Continue sessions using another Agent through
the ordinary Conversations page.

For example:

> Develop a todo notification app with add, complete and delete actions that
> survive refresh. Use an ordinary backend to show NomiFun connection status.
> Check its actual functions and configure preview and start commands.

Development uses the fixed built-in application Agent. The optional Agent called
by a running application is a separate choice in advanced settings.

## Import existing code or an artifact

**Import** opens the same workspace layout. Choose source or compiled artifact,
provide a folder/ZIP path on the NomiFun Host, or upload a ZIP from the current
computer. Desktop also provides folder and ZIP pickers. Files are copied into
the application's own directory while originals remain unchanged.

The right pane can import, configure and run an application **without a model
or Agent**. Alternatively, ask the built-in application Agent to inspect the
input, distinguish source from executable artifacts, and help fill settings.
Imports into an existing current application preserve its ID, directory and
development Session.

Detection uses package scripts, lockfiles and common entry points. Unsupported
stacks can be configured manually. Advanced settings include the relative project
directory, install/preview/build/foreground start commands, static output directory,
web address, Node path and environment variables. Optional commands may be empty.
Static output can use the ordinary HTTP preset. SSR, Python and background services
use their actual foreground command. Applications without a web UI need no URL.

## Preview, use and stop

First preview runs install and preview, or install/build/start when preview is
absent. First use runs install/build/start. Daily start runs only start. Rebuild
stops the old tree before install/build/start. Failures stop the sequence and
preserve actual errors and bounded command output.

The right pane loads an ordinary HTTP(S) application page on request. It can be
enlarged, collapsed, closed or opened in a browser. Closing a page does not stop
the program. Stopping generation and stopping an application are separate
operations. Preview and use share current files and data with one managed tree.

Embedded preview rejects the renderer's own origin and receives no NomiFun SDK,
connection credential or frontend Bridge. If a page forbids framing, open it in
a browser. Remote WebUI cannot treat the Host's localhost as the client computer's
address. Remote clients need a reachable application URL; there is no web proxy.

System Node is preferred, or select an existing executable. Node, npm and Bun are
detected independently. Missing Node links to the [official download page](https://nodejs.org/en/download)
without replacing an existing environment. Other tools report actual errors.

## Updating files and deleting

Conversation edits write the current directory. **Overwrite** copies external
files over it while preserving business data absent from the source; rebuild
afterwards. Import assistance detects commands again, while ordinary overwrite
preserves current settings. Both keep the application's identity.

Manual imports, overwrites and deletion check all current project sessions and are
rejected during an active development Turn or while effects remain pending or
unknown. Stop development and resolve the
original outcome before retrying. Reads, metadata/settings saves and application
stop remain available. An unconfirmed operation does not authorize automatic replay.

Delete stops the tree and removes owned source, artifacts, local data and registration.
Original imports and external data remain. Ordinary development Sessions and
valid canonical history remain: its navigation reference is not exclusive deletion
authority. Its former workspace path becomes unavailable after deletion; retained
history does not permit execution there.

There is no application version, deployment, publishing, business database migration,
rollback or special authoring acceptance workflow.

## Connecting to NomiFun

Preview/start backend processes automatically receive `NOMIFUN_HOST_URL`,
`NOMIFUN_LOCAL_TRUST` and optional `NOMIFUN_AGENT_BINDING_ID`. Users do not enter
tokens, install an SDK or pass an additional permission flow. Install/build and
static pages do not receive credentials. Web pages call platform REST operations
through their own ordinary backend:

```js
const response = await fetch(`${process.env.NOMIFUN_HOST_URL}/api/apps`, {
  headers: { 'x-nomi-local-trust': process.env.NOMIFUN_LOCAL_TRUST },
});
if (!response.ok) throw new Error(`NomiFun returned ${response.status}`);
const result = await response.json();
```

Keep these variables out of browser responses and logs. Host restart rotates
the credential, and the next start receives it. Agent tasks reuse the existing
[REST open/turn/observe/cancel API](remote-capability-api.zh.md); the platform
prepares the reference. Stop the application before changing its runtime target
Agent so the next process receives the correct configuration.

Applications can offer an ordinary MCP endpoint. Existing discovery and calls
require explicit Agent/Session source selection, never global injection. MCP
failures stay local and do not launch processes. Provider, Skill and theme
configuration keep their existing entry points.

## Using a personal Agent

A personal Agent can select **NomiFun application development** in the workbench
and choose the needed import, configuration, preview, rebuild and lifecycle actions.
Existing coding tools supply workspace/process authority. `read_app_guide` loads
instructions on demand. Only the built-in application template enables these
actions by default; other official templates do not. Existing personal selections
and frozen Sessions gain no actions automatically.

An external workspace can still be imported as a managed copy. Import does not
expand that Session's file access. Continue through the application's fixed
built-in workspace, or explicitly bind a regular Session to its directory.

Tauri and desktop WebUI support a minimum 880×600 viewport.
See [the architecture](../architecture/app-hosting.md).
