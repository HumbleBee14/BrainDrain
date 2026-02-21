import type { DeploymentStatus } from "@/lib/generated";

const colors: Record<DeploymentStatus, string> = {
    undeployed: "bg-zinc-800 text-zinc-500 border-zinc-700",
    deploying: "bg-amber-900/50 text-amber-400 border-amber-800",
    active: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
    inactive: "bg-red-900/50 text-red-400 border-red-800",
};

export function DeploymentStatusBadge({ status }: { status: DeploymentStatus }) {
    const cls = colors[status] ?? "bg-zinc-800 text-zinc-400 border-zinc-700";

    return (
        <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}>
            {status}
        </span>
    );
}
