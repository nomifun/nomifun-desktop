export const MAX_NOMI_MESSAGE_BUFFER_ENTRIES = 128;
export const MAX_NOMI_MESSAGE_BUFFER_BYTES = 2 * 1024 * 1024;
export const MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES = 512 * 1024;
export const MAX_NOMI_PROCESSED_CRON_IDS = 256;
export const MAX_NOMI_PENDING_POST_PROCESSES = 128;

const encoder = new TextEncoder();

export const isNomiTextReplacement = (message: {
  replace?: unknown;
  data?: unknown;
}): boolean => {
  if (message.replace === true) return true;
  const data = message.data;
  return (
    typeof data === 'object' &&
    data !== null &&
    !Array.isArray(data) &&
    (data as Record<string, unknown>).replace === true
  );
};

const byteLength = (value: string): number => encoder.encode(value).byteLength;

const takeUtf8Prefix = (value: string, maxBytes: number): string => {
  if (maxBytes <= 0 || value.length === 0) return '';
  if (byteLength(value) <= maxBytes) return value;

  let low = 0;
  let high = value.length;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    if (byteLength(value.slice(0, middle)) <= maxBytes) {
      low = middle;
    } else {
      high = middle - 1;
    }
  }

  // JavaScript slices by UTF-16 code units. A byte boundary can otherwise
  // retain only the high surrogate of a non-BMP scalar; TextEncoder would
  // measure that lone surrogate as U+FFFD and the fallback buffer would hold
  // corrupted text that never appeared in the stream.
  if (low > 0 && low < value.length) {
    const previous = value.charCodeAt(low - 1);
    const next = value.charCodeAt(low);
    if (previous >= 0xd800 && previous <= 0xdbff && next >= 0xdc00 && next <= 0xdfff) {
      low -= 1;
    }
  }
  return value.slice(0, low);
};

type StoredMessage = {
  content: string;
  turnId?: string;
  bytes: number;
  version: number;
  truncated: boolean;
};

export type TakenNomiMessageBuffer = {
  content: string;
  turnId?: string;
  version: number;
  /**
   * At least one fragment could not be retained in full. Consumers must not
   * derive a replacement projection from this buffer because doing so could
   * overwrite the complete rendered message with a bounded prefix.
   */
  truncated: boolean;
};

export type NomiMessageBufferEntry = TakenNomiMessageBuffer & {
  messageId: string;
};

/**
 * Bounded text storage for the local cron/display post-processor.
 *
 * The stream may be observed after a terminal frame or while a WebSocket
 * reconnect is in progress. This store deliberately separates:
 * - completed-message extraction (`take`),
 * - bounded eviction of inactive messages, and
 * - version-checked cleanup so an async processor cannot delete a newer
 *   fragment that arrived for the same message id.
 */
export class NomiMessageBufferStore {
  private readonly entries = new Map<string, StoredMessage>();
  private totalBytes = 0;
  private nextVersion = 0;

  get size(): number {
    return this.entries.size;
  }

  get byteSize(): number {
    return this.totalBytes;
  }

  has(messageId: string): boolean {
    return this.entries.has(messageId);
  }

  get(messageId: string): TakenNomiMessageBuffer | undefined {
    const entry = this.entries.get(messageId);
    return entry
      ? {
          content: entry.content,
          turnId: entry.turnId,
          version: entry.version,
          truncated: entry.truncated,
        }
      : undefined;
  }

  /**
   * Return the newest buffered segment for one logical turn. This is used only
   * by the legacy local post-processing fallback when an old terminal frame
   * does not carry the backend's explicit `final_text_msg_id` association.
   */
  findLatestForTurn(turnId: string): NomiMessageBufferEntry | undefined {
    let latest: NomiMessageBufferEntry | undefined;
    for (const [messageId, entry] of this.entries) {
      if (entry.turnId !== turnId) continue;
      if (!latest || entry.version > latest.version) {
        latest = {
          messageId,
          content: entry.content,
          turnId: entry.turnId,
          version: entry.version,
          truncated: entry.truncated,
        };
      }
    }
    return latest;
  }

  /**
   * Append a fragment. `isActive` protects buffers belonging to the current
   * live turn from eviction; inactive entries are evicted oldest-first.
   */
  append(
    messageId: string,
    chunk: string,
    turnId: string | undefined,
    isActive: (messageId: string, turnId?: string) => boolean
  ): boolean {
    if (!messageId || !chunk) return false;

    const existing = this.entries.get(messageId);
    const existingBytes = existing?.bytes ?? 0;
    const existingTurnId = turnId ?? existing?.turnId;

    const maxEntryBytes = MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES - existingBytes;
    if (maxEntryBytes <= 0) {
      if (existing) this.markTruncated(messageId, existing);
      return false;
    }
    const desiredChunk = takeUtf8Prefix(chunk, maxEntryBytes);
    if (!desiredChunk) {
      if (existing) this.markTruncated(messageId, existing);
      return false;
    }
    const desiredBytes = byteLength(desiredChunk);

    // Make room for a new entry and/or its next fragment. Inactive entries are
    // the only safe eviction candidates; if active buffers already consume the
    // full budget, fail closed rather than dropping live-turn content.
    const requiredEntrySlot = existing ? 0 : 1;
    this.evictInactive(
      isActive,
      messageId,
      requiredEntrySlot,
      desiredBytes
    );
    if (!existing && this.entries.size >= MAX_NOMI_MESSAGE_BUFFER_ENTRIES) return false;

    const availableBytes = Math.min(
      maxEntryBytes,
      Math.max(0, MAX_NOMI_MESSAGE_BUFFER_BYTES - this.totalBytes)
    );
    if (availableBytes <= 0) {
      if (existing) this.markTruncated(messageId, existing);
      return false;
    }

    const acceptedChunk = takeUtf8Prefix(desiredChunk, availableBytes);
    if (!acceptedChunk) {
      if (existing) this.markTruncated(messageId, existing);
      return false;
    }

    const content = `${existing?.content ?? ''}${acceptedChunk}`;
    const bytes = byteLength(content);
    const version = ++this.nextVersion;

    if (existing) {
      this.totalBytes -= existing.bytes;
      this.entries.delete(messageId);
    }
    this.entries.set(messageId, {
      content,
      turnId: existingTurnId,
      bytes,
      version,
      truncated: existing?.truncated === true || acceptedChunk !== chunk,
    });
    this.totalBytes += bytes;
    return true;
  }

  replace(
    messageId: string,
    content: string,
    turnId: string | undefined,
    isActive: (messageId: string, turnId?: string) => boolean
  ): boolean {
    if (!messageId) return false;

    const existing = this.entries.get(messageId);
    const existingBytes = existing?.bytes ?? 0;
    const existingTurnId = turnId ?? existing?.turnId;
    const requiredEntrySlot = existing ? 0 : 1;
    const desiredContent = takeUtf8Prefix(content, MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES);
    const desiredBytes = byteLength(desiredContent);
    const requiredAdditionalBytes = Math.max(0, desiredBytes - existingBytes);

    this.evictInactive(
      isActive,
      messageId,
      requiredEntrySlot,
      requiredAdditionalBytes
    );
    if (!existing && this.entries.size >= MAX_NOMI_MESSAGE_BUFFER_ENTRIES) return false;

    const availableBytes = Math.min(
      MAX_NOMI_MESSAGE_BUFFER_ENTRY_BYTES,
      Math.max(0, MAX_NOMI_MESSAGE_BUFFER_BYTES - (this.totalBytes - existingBytes))
    );
    const acceptedContent = takeUtf8Prefix(desiredContent, availableBytes);
    if (content && !acceptedContent) {
      if (existing) this.markTruncated(messageId, existing);
      return false;
    }

    if (existing) {
      this.totalBytes -= existing.bytes;
      this.entries.delete(messageId);
    }
    this.entries.set(messageId, {
      content: acceptedContent,
      turnId: existingTurnId,
      bytes: byteLength(acceptedContent),
      // An empty replacement still advances the version. This prevents an
      // in-flight processor from treating a hidden/cleared replacement as the
      // same buffer snapshot it observed before the replacement arrived.
      version: ++this.nextVersion,
      // A complete replacement is allowed to recover an earlier truncated
      // append sequence. A bounded replacement remains fail-closed.
      truncated: acceptedContent !== content,
    });
    this.totalBytes += byteLength(acceptedContent);
    return true;
  }

  take(messageId: string): TakenNomiMessageBuffer | undefined {
    const entry = this.entries.get(messageId);
    if (!entry) return undefined;
    this.entries.delete(messageId);
    this.totalBytes -= entry.bytes;
    return {
      content: entry.content,
      turnId: entry.turnId,
      version: entry.version,
      truncated: entry.truncated,
    };
  }

  /**
   * Remove only the entry observed by one async processor. A newer append for
   * the same message id survives, preventing a terminal cleanup race from
   * dropping late content.
   */
  deleteIfVersion(messageId: string, version: number): boolean {
    const entry = this.entries.get(messageId);
    if (!entry || entry.version !== version) return false;
    this.entries.delete(messageId);
    this.totalBytes -= entry.bytes;
    return true;
  }

  restore(
    messageId: string,
    buffer: TakenNomiMessageBuffer,
    isActive: (messageId: string, turnId?: string) => boolean
  ): boolean {
    if (this.entries.has(messageId)) return false;
    const restored = this.replace(messageId, buffer.content, buffer.turnId, isActive);
    if (restored && buffer.truncated) {
      const entry = this.entries.get(messageId);
      if (entry) entry.truncated = true;
    }
    return restored;
  }

  clear(messageId: string): void {
    const entry = this.entries.get(messageId);
    if (!entry) return;
    this.entries.delete(messageId);
    this.totalBytes -= entry.bytes;
  }

  clearTurn(turnId: string, keepMessageIds?: ReadonlySet<string>): void {
    for (const [messageId, entry] of this.entries) {
      if (entry.turnId !== turnId || keepMessageIds?.has(messageId)) continue;
      this.entries.delete(messageId);
      this.totalBytes -= entry.bytes;
    }
  }

  clearAll(): void {
    this.entries.clear();
    this.totalBytes = 0;
  }

  clearInactive(isActive: (messageId: string, turnId?: string) => boolean): void {
    for (const [messageId, entry] of this.entries) {
      if (!isActive(messageId, entry.turnId)) {
        this.entries.delete(messageId);
        this.totalBytes -= entry.bytes;
      }
    }
  }

  private markTruncated(messageId: string, entry: StoredMessage): void {
    this.entries.delete(messageId);
    this.entries.set(messageId, {
      ...entry,
      version: ++this.nextVersion,
      truncated: true,
    });
  }

  private evictInactive(
    isActive: (messageId: string, turnId?: string) => boolean,
    protectedMessageId: string | undefined,
    requiredEntrySlots: number,
    requiredBytes: number
  ): void {
    while (
      this.entries.size + requiredEntrySlots > MAX_NOMI_MESSAGE_BUFFER_ENTRIES ||
      this.totalBytes + requiredBytes > MAX_NOMI_MESSAGE_BUFFER_BYTES
    ) {
      let removed = false;
      for (const [messageId, entry] of this.entries) {
        if (messageId === protectedMessageId || isActive(messageId, entry.turnId)) continue;
        this.entries.delete(messageId);
        this.totalBytes -= entry.bytes;
        removed = true;
        break;
      }
      if (!removed) return;
    }
  }
}

export const rememberBoundedNomiCronId = (
  ids: Set<string>,
  messageId: string,
  maxSize = MAX_NOMI_PROCESSED_CRON_IDS
): void => {
  ids.delete(messageId);
  ids.add(messageId);
  while (ids.size > maxSize) {
    const oldest = ids.values().next().value;
    if (oldest === undefined) break;
    ids.delete(oldest);
  }
};

export const rememberBoundedNomiProcessedVersion = (
  versions: Map<string, number>,
  messageId: string,
  version: number,
  maxSize = MAX_NOMI_PROCESSED_CRON_IDS
): void => {
  versions.delete(messageId);
  versions.set(messageId, version);
  while (versions.size > maxSize) {
    const oldest = versions.keys().next().value;
    if (oldest === undefined) break;
    versions.delete(oldest);
  }
};
