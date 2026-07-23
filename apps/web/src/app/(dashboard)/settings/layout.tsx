"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useCurrentRole } from "@/hooks/use-team";

const tabs = [
  { label: "Team", href: "/settings/team" },
  { label: "LLM Provider", href: "/settings/llm" },
  { label: "Billing", href: "/settings/billing" },
  { label: "Usage", href: "/settings/usage" },
  { label: "Notifications", href: "/settings/notifications" },
  { label: "Inference", href: "/settings/inference" },
  { label: "Admin Config", href: "/settings/admin", adminOnly: true },
  { label: "Audit Log", href: "/settings/audit-log" },
];

export default function SettingsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const pathname = usePathname();
  const { isAdmin } = useCurrentRole();
  const visibleTabs = tabs.filter((tab) => !tab.adminOnly || isAdmin);

  return (
    <div>
      <div className="overflow-x-auto border-b border-zinc-200 dark:border-zinc-800 mb-6">
        <div className="flex gap-1 whitespace-nowrap">
          {visibleTabs.map((tab) => (
            <Link
              key={tab.href}
              href={tab.href}
              className={`shrink-0 px-3 md:px-4 py-2 text-xs md:text-sm font-medium transition ${
                pathname === tab.href
                  ? "text-zinc-900 dark:text-white border-b-2 border-emerald-500"
                  : "text-zinc-600 dark:text-zinc-400 hover:text-zinc-800 dark:hover:text-zinc-200"
              }`}
            >
              {tab.label}
            </Link>
          ))}
        </div>
      </div>
      {children}
    </div>
  );
}
