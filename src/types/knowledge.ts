/**
 * Knowledge-document domain types (P26 §二 页面二). Hand-written stand-ins for
 * the ts-rs generated types — same convention as `chat.ts`. Replaced by
 * `src/types/generated/*` once the P22 real interface lands.
 */

export interface KnowledgeDocument {
  id: string;
  /** File name as stored locally. */
  name: string;
  /** Epoch ms of import. */
  importedAt: number;
  /** Number of chunks the embedder produced. */
  chunkCount: number;
}
