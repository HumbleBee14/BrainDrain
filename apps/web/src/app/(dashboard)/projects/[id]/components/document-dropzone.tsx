"use client";

import { useCallback, useRef, useState } from "react";

interface UploadMutation {
  mutate: (files: File[]) => void;
  isPending: boolean;
  isError: boolean;
  error: Error | null;
}

export function DocumentDropzone({ uploadDocs }: { uploadDocs: UploadMutation }) {
  const [isDragging, setIsDragging] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleFiles = useCallback(
    (files: FileList | File[]) => {
      const fileArray = Array.from(files);
      if (fileArray.length > 0) {
        uploadDocs.mutate(fileArray);
      }
    },
    [uploadDocs],
  );

  return (
    <div
      className={`rounded-lg border-2 border-dashed p-4 text-center transition md:p-8 ${
        isDragging
          ? "border-violet-500 bg-violet-50 dark:bg-violet-900/10"
          : "border-zinc-300 hover:border-zinc-400 dark:border-zinc-700 dark:hover:border-zinc-600"
      }`}
      onDragOver={(e) => {
        e.preventDefault();
        setIsDragging(true);
      }}
      onDragLeave={() => setIsDragging(false)}
      onDrop={(e) => {
        e.preventDefault();
        setIsDragging(false);
        handleFiles(e.dataTransfer.files);
      }}
    >
      {uploadDocs.isPending ? (
        <p className="text-zinc-600 dark:text-zinc-400">Uploading...</p>
      ) : (
        <>
          <p className="mb-2 text-zinc-500">
            {isDragging
              ? "Drop files here"
              : "Drag and drop files here or click to upload"}
          </p>
          <p className="text-xs text-zinc-400 dark:text-zinc-600">
            Supports PDF, DOCX, TXT, HTML, MD, CSV (max 500 MB)
          </p>
          <label className="mt-4 inline-block cursor-pointer rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white transition hover:bg-violet-500">
            Choose Files
            <input
              ref={fileInputRef}
              type="file"
              multiple
              className="hidden"
              accept=".pdf,.docx,.txt,.html,.htm,.md,.csv"
              onChange={(e) => {
                if (e.target.files) handleFiles(e.target.files);
              }}
            />
          </label>
        </>
      )}
      {uploadDocs.isError && (
        <p className="mt-2 text-sm text-red-500 dark:text-red-400">
          {uploadDocs.error?.message}
        </p>
      )}
    </div>
  );
}
