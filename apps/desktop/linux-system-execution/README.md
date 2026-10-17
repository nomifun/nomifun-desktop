# Linux system execution installation

Linux administrator operations use `/usr/bin/pkexec --disable-internal-agent
/usr/libexec/nomifun-system-runner --stdio`. The main application and desktop
automation continue to run as the logged-in user. The runner is not setuid and is
not a persistent service. Its stdin/stdout carry the existing supervised process
protocol; neither stream is used to collect administrator passwords.

`com.nomifun.system-execution.run` binds polkit's `exec.path` to that one installed
runner and `exec.argv1` to `--stdio`. Active, inactive and sessionless callers all
use `auth_admin_keep`: the first administrator operation requires native
authentication, and subsequent operations for the same App subject and action
reuse polkit's own temporary authorization while it remains valid. There are no custom
polkit JavaScript rules, sudoers changes, saved passwords, or GUI root launches.
The distribution must provide polkit, `/usr/bin/pkexec`, and an authentication
agent for the actual login session. A working D-Bus or environment variable alone
does not prove authentication is available.

## Authentication without a graphical login session

A normal desktop session supplies its own polkit authentication agent. For an
explicitly started headless or WSL application without that agent, the user can
register a native agent for the actual NomiFun process from a separate real TTY:

```sh
pkttyagent --process APP_PID,START_TIME
```

Use the Linux application PID that launches pkexec and its `/proc/APP_PID/stat`
field 22 start time. The Windows launcher, a validation driver and the terminal
shell have different identities. Parse the stat record after its final `) `
before counting fields, because the process name can contain spaces. Keep the
agent terminal open and enter authentication only in its native prompt. End this
dedicated agent when the application or the test is finished. This registration
does not change the installed policy or retain a password.

On polkit 127, do not append `--fallback` for a process that has no logind session.
The [version 127 agent-selection implementation](https://github.com/polkit-org/polkit/blob/127/src/polkitbackend/polkitbackendinteractiveauthority.c#L1678-L1739)
can return before selecting the registered fallback when its session lookup
fails. An agent can therefore remain alive while pkexec reports that no agent
was found. This was observed with WSL processes in `init.scope`, for which
login1 `GetSessionByPID` returned no known session. A dedicated process agent
without `--fallback` is the applicable native entry in that environment. In a
real login session, `--fallback` can leave an existing session agent in control;
check the actual session and polkit version before selecting that option.

The same `auth_admin_keep` action applies when the App has no logind session.
After one successful native PAM authentication, later runner starts and readonly
acquisitions from that same App subject/action use the platform's temporary
authorization. Polkit owns its monotonic expiry and revokes it when the subject
disappears; an expired grant or a different App subject requires authentication
again. NomiFun does not cache credentials, retain a password, or maintain its own
authorization timer/database. The fixed runner path, `--stdio` binding and native
authentication agent continue to apply to every start.

## Native packages

`bun run build:linux` (also used by Linux `bun run build`) builds and freezes the
exact runner for each selected target. The generated bundle overlay places:

| Component | Installed path | Owner and mode |
| --- | --- | --- |
| Runner | `/usr/libexec/nomifun-system-runner` | `root:root`, `0755` |
| Action | `/usr/share/polkit-1/actions/com.nomifun.system-execution.policy` | `root:root`, `0644` |

Debian packages declare `pkexec`/`polkitd` (or the older `policykit-1` provider);
RPM packages declare `polkit`. Use the native package manager to install, update
and remove these assets with the application. Installing the package does not
grant an operation or prove that a graphical authentication agent is running.
The policy is discovered by polkit; no reload command, service registration, or
always-running root GUI is needed.

## Development and AppImage

Development and AppImage use the same fixed host installation. They cannot
execute a runner from an arbitrary checkout or from the AppImage mount through
pkexec. Build and review a matching runner before explicitly installing it:

```sh
# From the repository on Linux; the last line printed is the generated config.
bun scripts/prepare-linux-system-execution.mjs --profile debug
# The preceding command prints the unique staged directory on stderr.
sudo /absolute/staged/directory/install.sh
/absolute/staged/directory/install.sh --check
```

`bun run build:fast` also stages a debug runner and prints its installation
command. `bun run dev` never installs a system component automatically. Rebuild
and explicitly reinstall the staged runner when its protocol changes. If the
native NomiFun package owns the fixed files, use a matching package update; the
manual installer refuses to overwrite package-managed components.

AppImage packages carry a matching runner, this policy, installer and guide at
`usr/lib/nomifun-system-execution/` inside the image. Extract the image using its
standard `--appimage-extract` option, review the supplied files, then run:

```sh
sudo ./squashfs-root/usr/lib/nomifun-system-execution/install.sh
./squashfs-root/usr/lib/nomifun-system-execution/install.sh --check
```

The installer only copies the reviewed components into the fixed system paths;
it never installs polkit or requests/saves a password itself. Use the system's
native administrator authentication for this explicit installation. It rejects
symlink component sources and insecure system directories, and uses `restorecon`
when available to apply the distribution's own SELinux path labels. Remove a manual
installation with `sudo /absolute/staged/directory/install.sh --uninstall`.
An AppImage launch alone does not install or upgrade these host files. Install
the matching components when updating an AppImage, then verify a real operation.

`--check` verifies file ownership/modes and asks pkaction to describe the installed
action. It does not claim authenticated execution, cancellation, child cleanup,
readonly acquisition, or desktop permissions passed. Those require the native
product acceptance described in the Linux handoff.

Native mechanism references: [pkexec](https://polkit.pages.freedesktop.org/polkit/pkexec.1.html),
[polkit](https://polkit.pages.freedesktop.org/polkit/polkit.8.html),
[Tauri Debian files](https://v2.tauri.app/distribute/debian/#custom-files) and
[Tauri RPM configuration](https://v2.tauri.app/distribute/rpm/#configuring-the-rpm-package).
