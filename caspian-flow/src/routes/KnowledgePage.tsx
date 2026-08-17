import { useEffect, useRef, useState, type ChangeEvent } from "react";
import { Library, Plus, Trash2, FileText, Search } from "lucide-react";
import { useCaspian } from "@/hooks/useCaspian";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { formatRelative } from "@/lib/utils";
import type { KnowledgeDocument } from "@/types/knowledge";

/**
 * Knowledge base management (P26 §二 页面二). Mock data via `useCaspian`.
 *  - document list (name / imported time / chunk count) — dense rows
 *  - import button opens a real file picker; the selection is appended as a
 *    mock row (backend chunking/embedding lands in P22)
 *  - per-row delete (danger color)
 *  - stats: total docs / total chunks
 *  - search box is a placeholder (real retrieval in P22)
 */
export function KnowledgePage() {
  const caspian = useCaspian();
  const [docs, setDocs] = useState<KnowledgeDocument[]>([]);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    void caspian.listDocuments().then(setDocs);
  }, [caspian]);

  const totalChunks = docs.reduce((sum, d) => sum + d.chunkCount, 0);

  const onPick = async (e: ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (!files || files.length === 0) return;
    let current = docs;
    for (const f of Array.from(files)) {
      current = await caspian.importDocument(f.name);
    }
    setDocs(current);
    // Reset so picking the same file again still fires onChange.
    e.target.value = "";
  };

  const remove = async (id: string) => {
    const next = await caspian.deleteDocument(id);
    setDocs(next);
  };

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-4xl space-y-4 p-6">
        <header className="flex items-center gap-2">
          <Library size={18} className="text-accent" />
          <h1 className="text-base font-semibold text-foreground">知识库</h1>
          <span className="text-[12px] text-muted-foreground">
            {docs.length} 篇文档 · {totalChunks} 分块
          </span>
          <Button
            variant="primary"
            size="sm"
            className="ml-auto gap-1.5"
            onClick={() => fileRef.current?.click()}
          >
            <Plus size={14} />
            导入文档
          </Button>
          <input
            ref={fileRef}
            type="file"
            multiple
            hidden
            onChange={onPick}
          />
        </header>

        <div className="relative">
          <Search
            size={14}
            className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            placeholder="检索知识库（P22 接入后可用）"
            disabled
            className="pl-8 opacity-60"
          />
        </div>

        {docs.length === 0 ? (
          <p className="py-10 text-center text-[13px] text-muted-foreground">
            还没有导入任何文档
          </p>
        ) : (
          <ul className="flex flex-col border-t border-border">
            {docs.map((d) => (
              <li
                key={d.id}
                className="flex items-center gap-3 border-b border-border py-2"
              >
                <FileText size={15} className="shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate text-[13px] text-foreground">
                  {d.name}
                </span>
                <span className="shrink-0 text-[12px] text-muted-foreground">
                  {formatRelative(d.importedAt)}
                </span>
                <span className="shrink-0 text-[12px] tabular-nums text-muted-foreground">
                  {d.chunkCount} 分块
                </span>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={`删除 ${d.name}`}
                  onClick={() => void remove(d.id)}
                  className="shrink-0 text-danger hover:bg-danger/10 hover:text-danger"
                >
                  <Trash2 size={14} />
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
