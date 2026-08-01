/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Default Codex model list maintained by Nomi.
 *
 * PRE-HANDSHAKE FALLBACK ONLY: shown on the Guid page before the backend has
 * ever observed a codex session (agent_metadata.handshake.available_models is
 * still NULL). After the first session the persisted handshake catalog wins.
 * Validation is done by the codex bridge itself — Nomi only passes the id.
 *
 * Ids mirror what @agentclientprotocol/codex-acp@1.1.7 (bundled codex 0.145)
 * advertises: `<model>[<reasoning effort>]`. Keep this list SHORT — it is a
 * bootstrap hint, not a catalog.
 *
 * NOTE: deliberately no "first entry = default" semantics anymore. The
 * session's initial model comes from the user's local codex config
 * (CODEX_HOME config.toml), not from this list; entries here are only what
 * the user can explicitly pick before the first handshake.
 */
export const DEFAULT_CODEX_MODELS: Array<{ id: string; label: string; description: string }> = [
  {
    id: 'gpt-5.6-sol[medium]',
    label: 'GPT-5.6-Sol (medium)',
    description: 'Latest frontier agentic coding model',
  },
  {
    id: 'gpt-5.6-sol[high]',
    label: 'GPT-5.6-Sol (high)',
    description: 'Latest frontier agentic coding model, deeper reasoning',
  },
  {
    id: 'gpt-5.6-terra[medium]',
    label: 'GPT-5.6-Terra (medium)',
    description: 'Balanced agentic coding model for everyday work',
  },
  {
    id: 'gpt-5.6-luna[medium]',
    label: 'GPT-5.6-Luna (medium)',
    description: 'Fast and affordable agentic coding model',
  },
  {
    id: 'gpt-5.5[high]',
    label: 'GPT-5.5 (high)',
    description: 'Frontier model for complex coding, research, and real-world work',
  },
  {
    id: 'gpt-5.2[medium]',
    label: 'GPT-5.2 (medium)',
    description: 'Optimized for professional work and long-running agents',
  },
];
