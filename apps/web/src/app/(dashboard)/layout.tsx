import { UserButton } from "@clerk/nextjs";
import Link from "next/link";
import { ThemeToggle } from "@/components/theme-toggle";

export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="flex min-h-screen bg-white dark:bg-zinc-950">
      {/* Sidebar */}
      <aside className="w-64 border-r border-zinc-200 dark:border-zinc-800 p-6 flex flex-col">
        <Link href="/dashboard" className="text-xl font-bold text-zinc-900 dark:text-white mb-8">
          {process.env.NEXT_PUBLIC_APP_NAME || "Platform"}
        </Link>

        <nav className="flex flex-col gap-1 flex-1">
          <Link
            href="/dashboard"
            className="rounded-md px-3 py-2 text-sm text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-white hover:bg-zinc-100 dark:hover:bg-zinc-800 transition"
          >
            Dashboard
          </Link>
          <Link
            href="/projects"
            className="rounded-md px-3 py-2 text-sm text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-white hover:bg-zinc-100 dark:hover:bg-zinc-800 transition"
          >
            Projects
          </Link>
          <Link
            href="/settings/team"
            className="rounded-md px-3 py-2 text-sm text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-white hover:bg-zinc-100 dark:hover:bg-zinc-800 transition"
          >
            Settings
          </Link>
        </nav>

        <div className="flex items-center gap-3 pt-4 border-t border-zinc-200 dark:border-zinc-800">
          <UserButton afterSignOutUrl="/" />
          <ThemeToggle />
        </div>
      </aside>

      {/* Main content */}
      <main className="flex-1 p-8">{children}</main>
    </div>
  );
}
