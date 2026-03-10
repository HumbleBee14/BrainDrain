import type { Document } from "@/lib/api-client";
import { DocStatusBadge } from "./doc-status-badge";

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function DocumentRow({ doc }: { doc: Document }) {
  return (
    <div className="flex items-center justify-between py-3 px-4 border-b border-zinc-200 dark:border-zinc-800 last:border-b-0">
      <div className="flex items-center gap-3 min-w-0">
        <div className="min-w-0">
          <p className="text-sm text-zinc-900 dark:text-white truncate">
            {doc.filename}
          </p>
          <p className="text-xs text-zinc-400 dark:text-zinc-600">
            {formatFileSize(doc.file_size)}
            {doc.language && ` \u00b7 ${doc.language}`}
            {doc.page_count && ` \u00b7 ${doc.page_count} pages`}
          </p>
        </div>
      </div>
      <div className="flex items-center gap-3 shrink-0">
        {doc.parse_quality != null && (
          <span className="text-xs text-zinc-500">
            {(doc.parse_quality * 100).toFixed(0)}% quality
          </span>
        )}
        <DocStatusBadge status={doc.status} />
      </div>
    </div>
  );
}
