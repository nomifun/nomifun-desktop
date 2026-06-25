/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Curated catalog of the external CLI agents NomiFun knows how to detect.
 *
 * The backend (`/api/agents`) only returns agents whose CLI is actually
 * installed (it filters to `enabled && available`), and it carries no
 * homepage/install metadata. To let the UI surface *not-installed* agents —
 * so users can discover the full set of supported agents and install them —
 * we keep this small front-end catalog. The list mirrors the seeded
 * `agent_metadata` rows in 001_baseline.sql (ACP builtins + nanobot +
 * openclaw); `nomi` (internal, always available) and remote agents are
 * intentionally excluded.
 *
 * `backend` matches both the detected agent's `backend` key and the logo map
 * in `agentLogo.ts`. `website` is optional — when absent the card hides the
 * "manual install" button. `installHint` is a locale-neutral install command
 * embedded into the one-click install prompt; keep it empty unless the command
 * is product-owned, current, and low-risk enough for Nomi-assisted install.
 */
export interface SupportedAgent {
  /** Stable backend key — matches detected `agent.backend` and the logo map. */
  backend: string;
  /** Display name (mirrors the seeded `agent_metadata.name`). */
  name: string;
  /** Official site / docs for manual installation. Omit when unknown. */
  website?: string;
  /** Known install command, embedded into the one-click install prompt. Empty when unknown. */
  installHint: string;
  /** CLI binary used to verify a successful install (`<binary> --version`). */
  binary: string;
}

export const SUPPORTED_AGENTS: SupportedAgent[] = [
  {
    backend: 'claude',
    name: 'Claude Code',
    website: 'https://code.claude.com/docs/en/setup',
    installHint: 'curl -fsSL https://claude.ai/install.sh | bash',
    binary: 'claude',
  },
  {
    backend: 'codex',
    name: 'Codex CLI',
    website: 'https://developers.openai.com/codex/cli',
    installHint: 'curl -fsSL https://chatgpt.com/codex/install.sh | sh',
    binary: 'codex',
  },
  {
    backend: 'gemini',
    name: 'Gemini CLI',
    website: 'https://geminicli.com/',
    installHint: '',
    binary: 'gemini',
  },
  {
    backend: 'qwen',
    name: 'Qwen Code',
    website: 'https://qwenlm.github.io/qwen-code-docs/en/users/overview/',
    installHint: 'curl -fsSL https://qwen-code-assets.oss-cn-hangzhou.aliyuncs.com/installation/install-qwen.sh | bash',
    binary: 'qwen',
  },
  {
    backend: 'opencode',
    name: 'OpenCode',
    website: 'https://opencode.ai/docs/',
    installHint: 'curl -fsSL https://opencode.ai/install | bash',
    binary: 'opencode',
  },
  {
    backend: 'codebuddy',
    name: 'CodeBuddy',
    website: 'https://www.codebuddy.ai/cli',
    installHint: '',
    binary: 'codebuddy',
  },
  {
    backend: 'droid',
    name: 'Droid',
    website: 'https://docs.factory.ai/cli/getting-started/quickstart',
    installHint: '',
    binary: 'droid',
  },
  {
    backend: 'goose',
    name: 'Goose',
    website: 'https://goose-docs.ai/docs/getting-started/installation/',
    installHint: 'curl -fsSL https://github.com/aaif-goose/goose/releases/download/stable/download_cli.sh | bash',
    binary: 'goose',
  },
  {
    backend: 'auggie',
    name: 'Auggie',
    website: 'https://docs.augmentcode.com/cli/overview',
    installHint: '',
    binary: 'auggie',
  },
  {
    backend: 'kimi',
    name: 'Kimi Code',
    website: 'https://github.com/MoonshotAI/kimi-code',
    installHint: '',
    binary: 'kimi',
  },
  {
    backend: 'copilot',
    name: 'GitHub Copilot CLI',
    website: 'https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/install-copilot-cli',
    installHint: '',
    binary: 'copilot',
  },
  {
    backend: 'qoder',
    name: 'Qoder',
    website: 'https://docs.qoder.com/en/cli/quick-start',
    installHint: '',
    binary: 'qodercli',
  },
  {
    backend: 'vibe',
    name: 'Mistral Vibe',
    website: 'https://docs.mistral.ai/vibe/code/cli/install-setup',
    installHint: '',
    binary: 'vibe-acp',
  },
  {
    backend: 'cursor',
    name: 'Cursor CLI',
    website: 'https://cursor.com/docs/cli/installation',
    installHint: '',
    binary: 'agent',
  },
  {
    backend: 'kiro',
    name: 'Kiro',
    website: 'https://kiro.dev/docs/cli/',
    installHint: '',
    binary: 'kiro-cli',
  },
  {
    backend: 'hermes',
    name: 'Hermes Agent',
    website: 'https://hermes-agent.nousresearch.com/docs/getting-started/quickstart',
    installHint: '',
    binary: 'hermes',
  },
  {
    backend: 'snow',
    name: 'Snow',
    installHint: '',
    binary: 'snow',
  },
  {
    backend: 'nanobot',
    name: 'Nanobot',
    website: 'https://github.com/HKUDS/nanobot',
    installHint: '',
    binary: 'nanobot',
  },
  {
    backend: 'openclaw-gateway',
    name: 'OpenClaw',
    website: 'https://docs.openclaw.ai/install',
    installHint: '',
    binary: 'openclaw',
  },
];
