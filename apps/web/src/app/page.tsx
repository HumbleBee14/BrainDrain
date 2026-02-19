import Link from "next/link";

export default function LandingPage() {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center bg-zinc-950 text-white">
      <div className="max-w-2xl text-center px-6">
        <h1 className="text-5xl font-bold tracking-tight mb-4">
          {process.env.NEXT_PUBLIC_APP_NAME || "Platform"}
        </h1>
        <p className="text-xl text-zinc-400 mb-8">
          Fine-tune LLMs on your data. No technical knowledge required.
        </p>
        <p className="text-zinc-500 mb-12">
          Upload your documents, answer a few questions, and get a trained model
          deployed and ready to use.
        </p>
        <div className="flex gap-4 justify-center">
          <Link
            href="/sign-up"
            className="rounded-lg bg-white px-6 py-3 text-sm font-semibold text-zinc-950 hover:bg-zinc-200 transition"
          >
            Get Started
          </Link>
          <Link
            href="/sign-in"
            className="rounded-lg border border-zinc-700 px-6 py-3 text-sm font-semibold text-zinc-300 hover:border-zinc-500 transition"
          >
            Sign In
          </Link>
        </div>
      </div>
    </main>
  );
}
