"use client";

import { useState, useEffect } from "react";
import { UserButton } from "@clerk/nextjs";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { ThemeToggle } from "@/components/theme-toggle";
import { NotificationBell } from "@/components/notification-bell";

const appName = process.env.NEXT_PUBLIC_APP_NAME || "Platform";
// After sign-out, send users back to the public marketing site. Falls back to
// the app root (which routes to sign-in) when no marketing URL is configured.
const afterSignOutUrl = process.env.NEXT_PUBLIC_MARKETING_URL || "/";

const COLLAPSE_STORAGE_KEY = "sidebar-collapsed";

type NavLink = { href: string; label: string; icon: React.ReactNode };

const navLinks: NavLink[] = [
  {
    href: "/dashboard",
    label: "Dashboard",
    icon: (
      <>
        <rect x="3" y="3" width="7" height="9" rx="1" />
        <rect x="14" y="3" width="7" height="5" rx="1" />
        <rect x="14" y="12" width="7" height="9" rx="1" />
        <rect x="3" y="16" width="7" height="5" rx="1" />
      </>
    ),
  },
  {
    href: "/projects",
    label: "Projects",
    icon: (
      <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    ),
  },
  {
    href: "/settings/team",
    label: "Settings",
    icon: (
      <>
        <circle cx="12" cy="12" r="3" />
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82 1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
      </>
    ),
  },
];

function NavIcon({ children }: { children: React.ReactNode }) {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="shrink-0"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

/// Settings links point at a sub-route, so match on the top-level section or no
/// settings page would ever highlight.
function isActiveLink(pathname: string, href: string): boolean {
  const section = href.split("/").filter(Boolean)[0];
  return pathname === href || pathname.startsWith(`/${section}/`);
}

function NavLinks({
  pathname,
  collapsed = false,
  onNavigate,
}: {
  pathname: string;
  collapsed?: boolean;
  onNavigate?: () => void;
}) {
  return (
    <nav className="flex flex-1 flex-col gap-1">
      {navLinks.map((link) => {
        const active = isActiveLink(pathname, link.href);
        return (
          <Link
            key={link.href}
            href={link.href}
            onClick={onNavigate}
            aria-current={active ? "page" : undefined}
            title={collapsed ? link.label : undefined}
            className={`flex items-center gap-3 rounded-md px-3 py-2 text-sm transition ${
              collapsed ? "justify-center" : ""
            } ${
              active
                ? "bg-violet-50 font-medium text-violet-700 dark:bg-violet-500/10 dark:text-violet-300"
                : "text-zinc-600 hover:bg-zinc-100 hover:text-zinc-900 dark:text-zinc-400 dark:hover:bg-zinc-800 dark:hover:text-white"
            }`}
          >
            <NavIcon>{link.icon}</NavIcon>
            {!collapsed && link.label}
          </Link>
        );
      })}
    </nav>
  );
}

export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const pathname = usePathname();

  // Read after mount: localStorage is unavailable during SSR, so a lazy
  // useState initializer would hydrate with a mismatched sidebar width.
  useEffect(() => {
    setCollapsed(localStorage.getItem(COLLAPSE_STORAGE_KEY) === "true");
  }, []);

  const toggleCollapsed = () => {
    setCollapsed((prev) => {
      const next = !prev;
      localStorage.setItem(COLLAPSE_STORAGE_KEY, String(next));
      return next;
    });
  };

  // Close mobile menu on route change
  useEffect(() => {
    setMobileMenuOpen(false);
  }, [pathname]);

  return (
    // h-dvh + overflow-hidden confines scrolling to <main> so the sidebar stays
    // fixed instead of scrolling away with the page content.
    <div className="flex h-dvh overflow-hidden bg-white dark:bg-zinc-950">
      {/* Desktop sidebar */}
      <aside
        className={`hidden shrink-0 flex-col border-r border-zinc-200 p-4 transition-[width] duration-200 md:flex dark:border-zinc-800 ${
          collapsed ? "w-16" : "w-64"
        }`}
      >
        <div
          className={`mb-6 flex items-center ${
            collapsed ? "justify-center" : "justify-between"
          }`}
        >
          {!collapsed && (
            <Link
              href="/dashboard"
              className="truncate text-lg font-bold text-zinc-900 dark:text-white"
            >
              {appName}
            </Link>
          )}
          <button
            type="button"
            onClick={toggleCollapsed}
            className="rounded-md p-2 text-zinc-500 transition hover:bg-zinc-100 hover:text-zinc-900 dark:hover:bg-zinc-800 dark:hover:text-white"
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          >
            <svg
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden="true"
            >
              <rect x="3" y="4" width="18" height="16" rx="2" />
              <line x1="9" y1="4" x2="9" y2="20" />
            </svg>
          </button>
        </div>

        <NavLinks pathname={pathname} collapsed={collapsed} />

        <div
          className={`flex items-center gap-3 border-t border-zinc-200 pt-4 dark:border-zinc-800 ${
            collapsed ? "flex-col" : ""
          }`}
        >
          <UserButton afterSignOutUrl={afterSignOutUrl} />
          <ThemeToggle />
          <NotificationBell direction="up" />
        </div>
      </aside>

      {/* Mobile header */}
      <div className="fixed left-0 right-0 top-0 z-40 flex items-center justify-between border-b border-zinc-200 bg-white p-4 dark:border-zinc-800 dark:bg-zinc-950 md:hidden">
        <Link
          href="/dashboard"
          className="text-xl font-bold text-zinc-900 dark:text-white"
        >
          {appName}
        </Link>
        <div className="flex items-center gap-1">
          <NotificationBell direction="down" />
          <button
            type="button"
            onClick={() => setMobileMenuOpen(true)}
            className="rounded-md p-2 text-zinc-600 transition hover:bg-zinc-100 hover:text-zinc-900 dark:text-zinc-400 dark:hover:bg-zinc-800 dark:hover:text-white"
            aria-label="Open menu"
          >
            <svg
              width="24"
              height="24"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <line x1="3" y1="6" x2="21" y2="6" />
              <line x1="3" y1="12" x2="21" y2="12" />
              <line x1="3" y1="18" x2="21" y2="18" />
            </svg>
          </button>
        </div>
      </div>

      {/* Mobile sidebar overlay */}
      {mobileMenuOpen && (
        <>
          <div
            className="fixed inset-0 z-50 bg-black/50 md:hidden"
            onClick={() => setMobileMenuOpen(false)}
            aria-hidden="true"
          />

          <aside className="fixed inset-y-0 left-0 z-50 flex w-64 flex-col border-r border-zinc-200 bg-white p-6 dark:border-zinc-800 dark:bg-zinc-950 md:hidden">
            <div className="mb-8 flex items-center justify-between">
              <Link
                href="/dashboard"
                className="text-xl font-bold text-zinc-900 dark:text-white"
                onClick={() => setMobileMenuOpen(false)}
              >
                {appName}
              </Link>
              <button
                type="button"
                onClick={() => setMobileMenuOpen(false)}
                className="rounded-md p-2 text-zinc-600 transition hover:bg-zinc-100 hover:text-zinc-900 dark:text-zinc-400 dark:hover:bg-zinc-800 dark:hover:text-white"
                aria-label="Close menu"
              >
                <svg
                  width="24"
                  height="24"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <line x1="18" y1="6" x2="6" y2="18" />
                  <line x1="6" y1="6" x2="18" y2="18" />
                </svg>
              </button>
            </div>

            <NavLinks
              pathname={pathname}
              onNavigate={() => setMobileMenuOpen(false)}
            />

            <div className="flex items-center gap-3 border-t border-zinc-200 pt-4 dark:border-zinc-800">
              <UserButton afterSignOutUrl={afterSignOutUrl} />
              <ThemeToggle />
            </div>
          </aside>
        </>
      )}

      {/* Main content — the only scroll container */}
      <main className="flex-1 overflow-y-auto p-4 pt-20 md:p-8">
        {children}
      </main>
    </div>
  );
}
