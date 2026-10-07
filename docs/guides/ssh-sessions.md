# SSH remote sessions

SSH remote sessions let you hand NomiFun the connection details for a remote
Linux host and then work with the agent in an ordinary chat session while it
operates that host on your behalf — running commands, reading and editing
files, searching the tree, installing dependencies. Nothing runs on your local
machine: the whole session's work happens on the remote host.

> NomiFun is local-first and does not send your data anywhere except the LLM
> calls you configure. SSH is the one exception you opt into per host — an
> outbound connection to a machine you own. Credentials stay encrypted on this
> machine and are never returned to the UI in plaintext.

## The host book

Open **Settings → Remote hosts**. A host is a saved, reusable connection
profile owned by the installation owner:

- **Display name**, **Host**, **Port** (default 22), **Username**.
- **Authentication** — one of:
  - **Password**
  - **Private key** (PEM or OpenSSH format), with an optional **passphrase**
  - **Certificate** (an OpenSSH certificate plus its key)
  - **SSH agent** — auth is delegated to the ssh-agent already running on your
    machine; NomiFun never handles the key material
- **sudo password** (optional) — see [Sudo](#sudo) below.

All secret fields are encrypted at rest with AES-256-GCM. When you reopen a
host, stored secrets show as `***`; leave them untouched to keep the stored
value, or type a new value to replace it. Deleting a host is blocked while a
session is still bound to it.

**Test connection** dials the host and runs a trivial probe, reporting success
or the exact failure.

## Starting a session

From a host row, click **New session**. NomiFun creates a chat session bound to
that host and opens it. SSH sessions appear as their own kind in the session
list, separate from ordinary local work.

The default SSH session enables `ssh/exec`. Filesystem and sudo actions require
their corresponding Agent action grants and the bound host's exact resource
operation grants. When authorized, the tools operate the remote host:

- `ssh/exec` runs in a **persistent remote shell** — `cd`, `export`, activated
  virtualenvs and other shell state persist across commands within the session,
  exactly like a real interactive terminal. The shell runs as a non-root Linux
  account with the kernel's no-new-privileges bit enabled; `setpriv` support is
  required, and ordinary execution cannot elevate through setuid programs.
- `ssh/fs.read` and `ssh/fs.write` use **SFTP**, with atomic writes (temp file + rename)
  and preserved permissions. File edits are never built out of shell strings.
- The search and listing operations of `ssh/fs.read` search the remote tree
  (ripgrep if present, else grep).

A timeout interrupts the command and attempts to resynchronize the existing
shell, preserving its state when that succeeds. If the channel must be replaced,
or the transport reconnects, only the last working directory confirmed by a
completed command is restored. Environment variables, functions, and virtualenv
activation must be set again in the new process. An explicit `exit` reports the
server's exit status and the next command uses a fresh, equally restricted shell.

## The link, and how you can see it

One connection is held per session, by a backend pool that outlives the agent —
switching models tears the agent down and rebuilds it, and your remote shell,
its working directory and its environment all survive that untouched.

The session header carries a pill showing which host you are operating and the
live state of its link. Click it for `user@host:port`, the host key
fingerprint, and whether a sudo password is stored. The states are:

| State | Meaning |
| --- | --- |
| Connecting | dialling and authenticating |
| Connected | shell and SFTP are live |
| Shell recovering | the transport is fine, but a timed-out command left the shell unusable, so it is being reopened on the same connection |
| Reconnecting | the link dropped; redialling on a doubling backoff (1s to 60s), replaying the last proven working directory |
| Disconnected | the link is down. If it will not be retried — rejected credentials, or a changed host key — the pill says so, because retrying a rejected credential only locks the account out |
| Closed | the session's link is gone. It also reports whether the remote shell was **provably** reaped: if we could not confirm the remote shell exited, it says that rather than claiming a clean shutdown |

The sidebar groups SSH sessions by host, so a session bound to a machine is
always reachable from the list, not just right after you create it.

## Host keys

On the first connection to a host, its key is recorded in your own
`~/.ssh/known_hosts` (trust on first use, equivalent to OpenSSH's
`accept-new`). If a host's key later changes, the connection is blocked — this
is the normal defence against a man-in-the-middle, and it is not auto-accepted
because `known_hosts` is shared with your own `ssh`.

## Sudo

Privileged commands require both the Agent's `ssh/sudo` action and the bound
host's `sudo` operation. A stored password alone does not grant that authority.
The action accepts the command body, for example `systemctl restart nginx`.
The owner authenticates sudo on a separate short-lived shell, using an exact,
single-use password prompt, removes the password responder, and then executes
the body with `sudo -n`. The password never appears in command text, the
conversation transcript, or the request sent to the model. The ordinary
persistent shell never receives that password or elevated process state.
Leave the field blank if the remote account has passwordless sudo.

## Security posture

The Agent's frozen SSH action grants and exact host resource operations determine
which remote effects it may perform. Ordinary execution is fenced from privilege
elevation, while privileged execution requires the separate sudo grants. Be
deliberate about which hosts and operations you authorize and whether you store
a sudo password for production systems.

What NomiFun does guarantee: credentials are encrypted at rest, never returned
to the UI in plaintext, never placed in the conversation or the model request,
and host-key verification is enforced (accept-new on first use, blocked on
change).

## Not in this version

Importing hosts from `~/.ssh/config`, a live remote-output terminal panel,
ProxyJump/bastion hops, and MFA/keyboard-interactive auth are planned for later
phases. Remote targets are assumed to be POSIX Linux hosts.
