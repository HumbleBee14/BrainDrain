"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

const tabs = [
  { label: "Team", href: "/settings/team" },
  { label: "Billing", href: "/settings/billing" },
  { label: "Usage", href: "/settings/usage" },
  { label: "Notifications", href: "/settings/notifications" },
];

export default function SettingsLayout({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();

  return (
    <div>
      <div className="flex gap-1 border-b border-zinc-800 mb-6">
        {tabs.map((tab) => (
          <Link
            key={tab.href}
            href={tab.href}
            className={`px-4 py-2 text-sm font-medium transition ${
              pathname === tab.href
                ? "text-white border-b-2 border-emerald-500"
                : "text-zinc-400 hover:text-zinc-200"
            }`}
          >
            {tab.label}
          </Link>
        ))}
      </div>
      {children}
    </div>
  );
}
