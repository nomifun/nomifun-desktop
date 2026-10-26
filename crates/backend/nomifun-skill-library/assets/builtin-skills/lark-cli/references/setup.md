# Installation and authorization

Use the existing NomiFun process tools and environment for every command here.
First follow [shared installation and executable discovery](installation.md).
Every `lark-cli` example below means the selected absolute launcher, passed
as `command` with literal `args`. Authentication errors do not require another
installation.

## Reuse configuration

Account setup can be managed in **Office → Connection settings**
(`/office`). The workbench's selected connection is not an implicit
selector for Agent process calls. Inspect `profile list`, use the profile the
user selected, and pass `--profile <name>` explicitly to subsequent business
and auth commands. Do not mutate the global default with `profile use`; if the
intended profile is ambiguous, resolve that identity before reading or writing.

Check `lark-cli auth status` when setting up or diagnosing authentication.
Use the installed `config --help` / `auth --help` to diagnose a failure;
an expired user login does not require replacing the configured application.
Do not reset a workspace or overwrite an existing application's credentials.

Only when no application is configured (or the user requests a new one), run:

```text
lark-cli config init --new --name <new-profile>
```

Start this as a supervised process with a short initial wait and enough total
time for browser authorization. Read its output while it is running and
surface the configuration link immediately. The user completes the browser
steps; then observe the same process to its final status. If interaction needs
another turn, hand control back with the link and retain the process handle.
Do not wait silently until the link expires or start a second setup process.

For configuration and login URLs, forward the exact CLI-returned URL without
rebuilding its query. Inspect `lark-cli auth qrcode --help`, generate a PNG with
`lark-cli auth qrcode <returned-url> --output ./lark-auth.png`, and show it
alongside the link using the available artifact/file display facility. Keep
the output in the task directory. Never invent an authorization URL or ask
the user to paste a secret into chat.

## User login without blocking link delivery

Choose only the scopes needed by the task. Inspect the relevant command help
or `lark-cli schema <service>.<resource>.<method>` for its actual scopes.
Inspect `lark-cli auth login --help` for the installed version. For example:

```text
lark-cli auth login --domain calendar --no-wait --json
```

For an exact permission use `--scope <required-scope>` instead of `--domain`.
Always specify the intended scope/domain: current versions treat both bare
`auth login` and `--recommend` as requesting all known business domains.

The split login returns a verification URL and `device_code`. Show the URL
and QR code, explain that the user must finish in the browser and reply when
done, and hand control back. Keep the device code only for this in-progress
authorization flow; do not publish it or save it into skill resources.
Do not start a blocking poll before the user has received the link.

After the user reports completion, finish the same flow yourself:

```text
lark-cli auth login --device-code <device-code-from-this-flow>
lark-cli auth status --json --verify
```

Observe the final result before resuming the original task. On expiration,
restart with the same required scope/domain selection to obtain a fresh link;
do not reuse the expired code. On cancellation or denial, stop that flow.

## Permission failures and identity

- User identity needs both application scopes and user consent. Use the
  reported `missing_scopes` and the installed schema to request only the
  applicable missing permission, then retry the original operation once
  authorization succeeds.
- Bot identity needs application permissions in the developer console;
  `auth login` is not a bot-permission repair. Surface the returned
  `console_url` for the user to configure the application.
- Resource membership/ACL errors can persist with valid scopes. Explain the
  specific missing access; do not change sharing or switch identities without
  the user's direction.
- Preserve the user's existing Feishu/Lark application and region settings.
  Account replacement, logout and revocation are separate requested actions.

Upstream references: [quick start](https://github.com/larksuite/cli/blob/main/README.zh.md),
[identity and permissions](https://github.com/larksuite/cli/blob/main/skills/lark-shared/references/lark-shared-identity-and-permissions.md).
