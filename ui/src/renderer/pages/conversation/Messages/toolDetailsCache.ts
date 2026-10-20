import { database } from '@/common/adapter/ipcBridge';
import { toolDetailSnapshotKey, type ToolDetailPage, type ToolDetailRequest } from '@/common/chat/toolDetails';

/** A viewer-owned, bounded read cache. No payload is written into TMessage or storage. */
export class ToolDetailsCache {
  private pages = new Map<string, Promise<ToolDetailPage>>();
  constructor(private readonly read = (request: ToolDetailRequest) => database.getToolDetails.invoke(request)) {}

  get(request: ToolDetailRequest): Promise<ToolDetailPage> {
    const key = `${toolDetailSnapshotKey(request)}:${request.section}:${request.cursor ?? ''}`;
    const cached = this.pages.get(key);
    if (cached) return cached;
    const page = this.read({ ...request, limit: 16_384 }).then(value => {
      if (value.turn_id !== request.turn_id || value.call_id !== request.call_id || value.section !== request.section
        || (request.through_seq !== undefined && value.through_seq !== request.through_seq)
        || (request.expected_result_seq !== undefined && value.result_seq !== request.expected_result_seq)
        || value.content.length > 65_536) throw new Error('Tool detail identity did not match its source');
      // A pending observation must be refreshed when the user opens it again.
      if (value.availability !== 'available') this.pages.delete(key);
      return value;
    }).catch(error => { this.pages.delete(key); throw error; });
    this.pages.set(key, page);
    while (this.pages.size > 32) this.pages.delete(this.pages.keys().next().value!);
    return page;
  }
  clear(): void { this.pages.clear(); }
}
