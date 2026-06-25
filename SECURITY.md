# Security Policy

NomiFun can execute local tools, shell commands, browser automation, desktop
automation, and remote capability calls. Treat an authenticated NomiFun instance
as a high-privilege local automation surface.

## Reporting Vulnerabilities

Please report suspected vulnerabilities privately before opening a public issue.
If the project has not published a dedicated security contact yet, contact the
maintainers through the repository owner channel and include:

- affected version or commit,
- operating system and deployment mode (`nomifun-desktop`, `nomifun-web`, or
  standalone `nomicore`),
- reproduction steps,
- impact assessment,
- logs or screenshots with secrets redacted.

Do not include live tokens, passwords, provider keys, private conversation
content, or proprietary workspace files in reports.

## Supported Versions

The project is pre-1.0. Security fixes target the current default branch unless
a release branch explicitly says it is supported.

## Deployment Guidance

- Do not expose the embedded desktop backend port directly. Use WebUI Remote
  Access or `nomifun-web`, both of which provide authenticated surfaces.
- Use TLS when exposing `nomifun-web` or remote capability APIs over a network.
- Treat companion access tokens as full-control credentials for the scoped
  companion and its enabled capabilities.
- Prefer least-privilege provider keys, MCP servers, and workspace paths.
- Review full-auto terminal permissions before binding them to AutoWork.

See [docs/reference/troubleshooting.md](docs/reference/troubleshooting.md) and
[docs/guides/remote-capability-api.md](docs/guides/remote-capability-api.md)
for related operational details.
