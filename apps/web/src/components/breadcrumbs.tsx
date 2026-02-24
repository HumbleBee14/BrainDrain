"use client";

import Link from "next/link";

export interface BreadcrumbItem {
  label: string;
  href?: string;
}

export function Breadcrumbs({ items }: { items: BreadcrumbItem[] }) {
  return (
    <nav className="flex flex-wrap items-center gap-1.5 text-xs md:text-sm text-zinc-500 mb-3 md:mb-4">
      {items.map((item, i) => {
        const isLast = i === items.length - 1;
        return (
          <span key={i} className="flex items-center gap-1.5">
            {i > 0 && <span className="text-zinc-300 dark:text-zinc-700">/</span>}
            {isLast || !item.href ? (
              <span className={isLast ? "text-zinc-700 dark:text-zinc-300" : ""}>
                {item.label}
              </span>
            ) : (
              <Link href={item.href} className="truncate max-w-[150px] md:max-w-none inline-block hover:text-zinc-700 dark:hover:text-zinc-300 transition">
                {item.label}
              </Link>
            )}
          </span>
        );
      })}
    </nav>
  );
}
