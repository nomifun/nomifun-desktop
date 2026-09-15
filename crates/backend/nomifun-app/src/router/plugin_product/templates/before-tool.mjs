// A business rule example, not a substitute for permissions or a shell sandbox.
// The host validates and redacts arguments before this check. Never patch input.
export async function start() {
  return {
    async invoke({ method, payload, signal }) {
      if (signal.aborted) throw new Error('Check cancelled');
      if (method !== 'agent.before_tool' || payload.phase !== 'before_tool') {
        throw new Error('Unsupported check phase');
      }
      const referencesSensitiveFile = value => {
        if (typeof value === 'string') return /(?:^|[\\/\s'"=])(?:\.env(?:\.[\w-]+)?|id_rsa|id_ed25519)(?:$|[\s'";|&])/i.test(value);
        if (Array.isArray(value)) return value.some(referencesSensitiveFile);
        return value !== null && typeof value === 'object' && Object.values(value).some(referencesSensitiveFile);
      };
      if (referencesSensitiveFile(payload.arguments)) {
        return { decision: 'deny', reason: '敏感文件检查已阻止此操作。请使用不含凭据的示例文件，或先调整 Agent 中已选择的检查规则。目标工具未执行。' };
      }
      return { decision: 'allow' };
    },
    async dispose() {},
  };
}
