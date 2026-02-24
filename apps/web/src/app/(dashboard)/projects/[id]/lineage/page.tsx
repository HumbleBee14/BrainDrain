"use client";

import { useParams } from "next/navigation";
import { useMemo } from "react";
import Link from "next/link";
import { useProject } from "@/hooks/use-projects";
import { useDocuments } from "@/hooks/use-documents";
import { useDatasets } from "@/hooks/use-datasets";
import { useTrainingJobs } from "@/hooks/use-training";
import { useModels } from "@/hooks/use-models";
import { Breadcrumbs } from "@/components/breadcrumbs";
import type {
  Document,
  Dataset,
  TrainingJob,
  Model,
} from "@/lib/api-client";

/** Build the lineage graph from raw entities using FK relationships. */
function buildLineage(
  documents: Document[],
  datasets: Dataset[],
  jobs: TrainingJob[],
  models: Model[],
) {
  // TrainingJob.dataset_id → Dataset
  const datasetToJobs = new Map<string, TrainingJob[]>();
  for (const job of jobs) {
    const list = datasetToJobs.get(job.dataset_id) ?? [];
    list.push(job);
    datasetToJobs.set(job.dataset_id, list);
  }

  // Model.training_job_id → TrainingJob
  const jobToModels = new Map<string, Model[]>();
  for (const model of models) {
    const list = jobToModels.get(model.training_job_id) ?? [];
    list.push(model);
    jobToModels.set(model.training_job_id, list);
  }

  return { datasetToJobs, jobToModels };
}

const STATUS_COLORS: Record<string, string> = {
  // Documents
  uploaded: "bg-zinc-700 text-zinc-300",
  parsing: "bg-blue-900/50 text-blue-400",
  parsed: "bg-emerald-900/50 text-emerald-400",
  // Datasets
  generating: "bg-blue-900/50 text-blue-400",
  review_pending: "bg-amber-900/50 text-amber-400",
  approved: "bg-emerald-900/50 text-emerald-400",
  // Training Jobs
  pending: "bg-zinc-700 text-zinc-300",
  cost_approval: "bg-amber-900/50 text-amber-400",
  training: "bg-violet-900/50 text-violet-400",
  completed: "bg-emerald-900/50 text-emerald-400",
  // Models
  undeployed: "bg-zinc-700 text-zinc-300",
  deploying: "bg-blue-900/50 text-blue-400",
  active: "bg-emerald-900/50 text-emerald-400",
  // General
  failed: "bg-red-900/50 text-red-400",
  cancelled: "bg-zinc-800 text-zinc-500",
  archived: "bg-zinc-800 text-zinc-500",
};

function StatusDot({ status }: { status: string }) {
  const cls = STATUS_COLORS[status] ?? "bg-zinc-700 text-zinc-400";
  return (
    <span
      className={`inline-block w-2 h-2 rounded-full ${cls.split(" ")[0].replace("/50", "")}`}
      title={status}
    />
  );
}

function StatusBadge({ status }: { status: string }) {
  const cls = STATUS_COLORS[status] ?? "bg-zinc-800 text-zinc-400";
  return (
    <span className={`inline-flex px-1.5 py-0.5 rounded text-[10px] font-medium ${cls}`}>
      {status.replace(/_/g, " ")}
    </span>
  );
}

function Connector({ active }: { active?: boolean }) {
  return (
    <div className="flex items-center justify-center py-1">
      <div
        className={`w-0.5 h-6 ${active ? "bg-emerald-700" : "bg-zinc-700"}`}
      />
    </div>
  );
}

function HorizontalConnector({ active }: { active?: boolean }) {
  return (
    <div className="flex items-center px-2">
      <div
        className={`h-0.5 w-6 ${active ? "bg-emerald-700" : "bg-zinc-700"}`}
      />
      <div
        className={`w-0 h-0 border-t-4 border-b-4 border-l-4 border-t-transparent border-b-transparent ${active ? "border-l-emerald-700" : "border-l-zinc-700"}`}
      />
    </div>
  );
}

function StageHeader({
  label,
  count,
  color,
}: {
  label: string;
  count: number;
  color: string;
}) {
  return (
    <div className="flex items-center gap-2 mb-3">
      <div className={`w-3 h-3 rounded-sm ${color}`} />
      <h3 className="text-sm font-semibold text-zinc-300 uppercase tracking-wider">
        {label}
      </h3>
      <span className="text-xs text-zinc-600">({count})</span>
    </div>
  );
}

export default function LineagePage() {
  const params = useParams<{ id: string }>();
  const { data: project } = useProject(params.id);
  const { data: docsData } = useDocuments(params.id, 0, 100);
  const { data: datasetsData } = useDatasets(params.id);
  const { data: jobsData } = useTrainingJobs(params.id);
  const { data: modelsData } = useModels(params.id, 0, 50);

  const documents = useMemo(() => docsData?.data ?? [], [docsData?.data]);
  const datasets = useMemo(() => datasetsData?.data ?? [], [datasetsData?.data]);
  const jobs = useMemo(() => jobsData?.data ?? [], [jobsData?.data]);
  const models = useMemo(() => modelsData?.data ?? [], [modelsData?.data]);

  const { datasetToJobs } = useMemo(
    () => buildLineage(documents, datasets, jobs, models),
    [documents, datasets, jobs, models],
  );

  const isLoading = !docsData || !datasetsData || !jobsData || !modelsData;

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading lineage data...</p>
      </div>
    );
  }

  // Trace full paths: for each model, trace back to its training job → dataset → documents
  const hasData =
    documents.length > 0 ||
    datasets.length > 0 ||
    jobs.length > 0 ||
    models.length > 0;

  return (
    <div>
      <div className="mb-8">
        <Breadcrumbs
          items={[
            { label: "Projects", href: "/projects" },
            {
              label: project?.name || "Project",
              href: `/projects/${params.id}`,
            },
            { label: "Data Lineage" },
          ]}
        />
        <h1 className="text-2xl font-bold text-white">Data Lineage</h1>
        <p className="text-sm text-zinc-500 mt-1">
          Trace data flow from documents through training to deployed models
        </p>
      </div>

      {!hasData ? (
        <div className="rounded-lg border border-zinc-800 p-8 text-center">
          <p className="text-zinc-500">
            No pipeline data yet. Upload documents to get started.
          </p>
        </div>
      ) : (
        <div className="space-y-2">
          {/* Stage 1: Documents */}
          <div className="rounded-lg border border-zinc-800 p-4">
            <StageHeader
              label="Documents"
              count={documents.length}
              color="bg-blue-500"
            />
            <div className="flex flex-wrap gap-2">
              {documents.map((doc) => (
                <div
                  key={doc.id}
                  className="flex items-center gap-1.5 rounded-md border border-zinc-700 bg-zinc-900 px-2.5 py-1.5 text-xs"
                >
                  <StatusDot status={doc.status} />
                  <span className="text-zinc-300 max-w-[200px] truncate">
                    {doc.filename}
                  </span>
                  <StatusBadge status={doc.status} />
                </div>
              ))}
            </div>
          </div>

          <Connector active={datasets.length > 0} />

          {/* Stage 2: Datasets */}
          <div className="rounded-lg border border-zinc-800 p-4">
            <StageHeader
              label="Datasets"
              count={datasets.length}
              color="bg-amber-500"
            />
            {datasets.length === 0 ? (
              <p className="text-xs text-zinc-600">
                No datasets generated yet.
              </p>
            ) : (
              <div className="space-y-3">
                {datasets.map((ds) => {
                  const linkedJobs = datasetToJobs.get(ds.id) ?? [];
                  return (
                    <div key={ds.id}>
                      <div className="flex items-center gap-2">
                        <Link
                          href={`/projects/${params.id}/dataset?datasetId=${ds.id}`}
                          className="flex items-center gap-1.5 rounded-md border border-zinc-700 bg-zinc-900 px-2.5 py-1.5 text-xs hover:border-zinc-600 transition"
                        >
                          <StatusDot status={ds.status} />
                          <span className="text-zinc-300">{ds.name}</span>
                          {ds.pair_count != null && (
                            <span className="text-zinc-600">
                              {ds.pair_count} pairs
                            </span>
                          )}
                          <StatusBadge status={ds.status} />
                        </Link>
                        {linkedJobs.length > 0 && (
                          <>
                            <HorizontalConnector active />
                            <div className="flex flex-wrap gap-1.5">
                              {linkedJobs.map((job) => (
                                <Link
                                  key={job.id}
                                  href={`/projects/${params.id}/training/${job.id}`}
                                  className="flex items-center gap-1 rounded-md border border-violet-800/50 bg-violet-900/10 px-2 py-1 text-[10px] hover:border-violet-700 transition"
                                >
                                  <StatusDot status={job.status} />
                                  <span className="text-violet-300">
                                    {job.base_model.split("/").pop()}
                                  </span>
                                  <span className="text-violet-500">
                                    {String(job.method).toUpperCase()}
                                  </span>
                                  <StatusBadge status={job.status} />
                                </Link>
                              ))}
                            </div>
                          </>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          <Connector active={jobs.length > 0} />

          {/* Stage 3: Training Jobs (those not linked to any dataset — orphans) */}
          {jobs.filter(
            (j) => !datasets.some((ds) => ds.id === j.dataset_id),
          ).length > 0 && (
            <>
              <div className="rounded-lg border border-dashed border-zinc-700 p-4">
                <StageHeader
                  label="Unlinked Training Jobs"
                  count={
                    jobs.filter(
                      (j) => !datasets.some((ds) => ds.id === j.dataset_id),
                    ).length
                  }
                  color="bg-violet-500"
                />
                <div className="flex flex-wrap gap-2">
                  {jobs
                    .filter(
                      (j) => !datasets.some((ds) => ds.id === j.dataset_id),
                    )
                    .map((job) => (
                      <Link
                        key={job.id}
                        href={`/projects/${params.id}/training/${job.id}`}
                        className="flex items-center gap-1.5 rounded-md border border-zinc-700 bg-zinc-900 px-2.5 py-1.5 text-xs hover:border-zinc-600 transition"
                      >
                        <StatusDot status={job.status} />
                        <span className="text-zinc-300">
                          {job.base_model.split("/").pop()}
                        </span>
                        <StatusBadge status={job.status} />
                      </Link>
                    ))}
                </div>
              </div>
              <Connector active={models.length > 0} />
            </>
          )}

          {/* Stage 4: Models */}
          <div className="rounded-lg border border-zinc-800 p-4">
            <StageHeader
              label="Models"
              count={models.length}
              color="bg-emerald-500"
            />
            {models.length === 0 ? (
              <p className="text-xs text-zinc-600">
                No models produced yet.
              </p>
            ) : (
              <div className="space-y-3">
                {models.map((model) => {
                  const parentJob = jobs.find(
                    (j) => j.id === model.training_job_id,
                  );
                  const parentDataset = parentJob
                    ? datasets.find((ds) => ds.id === parentJob.dataset_id)
                    : null;

                  return (
                    <div key={model.id} className="flex items-start gap-2">
                      <Link
                        href={`/projects/${params.id}/models/${model.id}`}
                        className="flex items-center gap-1.5 rounded-md border border-zinc-700 bg-zinc-900 px-2.5 py-1.5 text-xs hover:border-zinc-600 transition"
                      >
                        <StatusDot status={model.deployment_status} />
                        <span className="text-zinc-300">{model.name}</span>
                        <span className="text-zinc-600">v{model.version}</span>
                        <StatusBadge status={model.deployment_status} />
                      </Link>
                      <div className="text-[10px] text-zinc-600 pt-1">
                        {parentJob && (
                          <span>
                            from{" "}
                            <span className="text-violet-400">
                              {parentJob.base_model.split("/").pop()}
                            </span>
                            {parentDataset && (
                              <>
                                {" "}
                                on{" "}
                                <span className="text-amber-400">
                                  {parentDataset.name}
                                </span>
                              </>
                            )}
                          </span>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {/* Legend */}
          <div className="flex flex-wrap gap-4 mt-6 pt-4 border-t border-zinc-800">
            {[
              { label: "Pending", cls: "bg-zinc-600" },
              { label: "In Progress", cls: "bg-blue-500" },
              { label: "Review", cls: "bg-amber-500" },
              { label: "Completed", cls: "bg-emerald-500" },
              { label: "Failed", cls: "bg-red-500" },
              { label: "Deployed", cls: "bg-emerald-400" },
            ].map((item) => (
              <div key={item.label} className="flex items-center gap-1.5">
                <div className={`w-2 h-2 rounded-full ${item.cls}`} />
                <span className="text-[10px] text-zinc-500">
                  {item.label}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
