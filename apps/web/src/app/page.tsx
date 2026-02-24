import Link from "next/link";
import { auth } from "@clerk/nextjs/server";
import { redirect } from "next/navigation";

export default async function LandingPage() {
  const { userId } = await auth();

  if (userId) {
    redirect("/dashboard");
  }

  return (
    <main className="flex min-h-screen flex-col items-center justify-center bg-white text-zinc-950 dark:bg-zinc-950 dark:text-white">
      <div className="max-w-2xl text-center px-6">
        <h1 className="text-5xl font-bold tracking-tight mb-4">
          {process.env.NEXT_PUBLIC_APP_NAME || "Platform"}
        </h1>
        <p className="text-xl text-zinc-600 dark:text-zinc-400 mb-8">
          Fine-tune LLMs on your data. No technical knowledge required.
        </p>
        <p className="text-zinc-500 mb-12">
          Upload your documents, answer a few questions, and get a trained model
          deployed and ready to use.
        </p>
        <div className="flex gap-4 justify-center">
          <Link
            href="/sign-up"
            className="rounded-lg bg-zinc-900 text-white hover:bg-zinc-800 dark:bg-white dark:text-zinc-950 dark:hover:bg-zinc-200 px-6 py-3 text-sm font-semibold transition"
          >
            Get Started
          </Link>
          <Link
            href="/sign-in"
            className="rounded-lg border border-zinc-300 dark:border-zinc-700 px-6 py-3 text-sm font-semibold text-zinc-700 dark:text-zinc-300 hover:border-zinc-500 transition"
          >
            Sign In
          </Link>
        </div>
      </div>
    </main>
  );
}
