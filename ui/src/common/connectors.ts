/** Connector registry: third-party account connections managed under MCP & 技能 → 连接器. */
export type ConnectorId = 'feishu';

export interface ConnectorDefinition {
  id: ConnectorId;
  /** Skill names surfaced through this connector instead of the ordinary skills list. */
  providedSkills: string[];
  /** Route of the knowledge space this connector powers, if any. */
  knowledgePath: string | null;
}

export const CONNECTORS: ConnectorDefinition[] = [
  { id: 'feishu', providedSkills: ['lark-cli'], knowledgePath: '/knowledge/feishu' },
];

const CONNECTOR_PROVIDED_SKILLS = new Set(CONNECTORS.flatMap(connector => connector.providedSkills));

/** Skills owned by a connector are hidden from the ordinary skills grid. */
export const isConnectorProvidedSkill = (name: string): boolean => CONNECTOR_PROVIDED_SKILLS.has(name);
