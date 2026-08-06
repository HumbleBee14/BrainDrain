"use client";

import { useParams } from "next/navigation";
import Link from "next/link";
import { toast } from "sonner";
import { useEffect, useMemo, useState } from "react";
import { useProject } from "@/hooks/use-projects";
import { useDocuments, useUploadDocuments } from "@/hooks/use-documents";
import {
  usePipelineStatus,
  useTriggerParse,
  useTriggerRefine,
} from "@/hooks/use-pipeline";
import { useDatasets } from "@/hooks/use-datasets";
import { useTrainingJobs } from "@/hooks/use-training";
import { useModels } from "@/hooks/use-models";
import { useModelCatalog } from "@/hooks/use-catalog";
import { useOnboarding } from "@/hooks/use-onboarding";
import { Breadcrumbs } from "@/components/breadcrumbs";
import { Button } from "@/components/ui/button";
import { TabBar } from "@/components/ui/tabs";
import { StatusBadge } from "./components";
import {
  PipelineStepper,
  computePipelineSteps,
} from "./components/pipeline-stepper";
import { NextActionCard } from "./components/next-action-card";
import { RouteExplainer } from "./components/route-explainer";
import { DocumentDropzone } from "./components/document-dropzone";
import { DocumentsTab } from "./components/documents-tab";
import { DatasetsTab } from "./components/datasets-tab";
import { TrainingTab } from "./components/training-tab";
import { ModelsTab } from "./components/models-tab";
import { SettingsTab } from "./components/settings-tab";
import { DistillSetup } from "./components/distill-setup";
import { OneClickSetup } from "./components/one-click-setup";

const FALLBACK_BASE_MODEL = "unsloth/Llama-3.2-1B-Instruct";

type TabKey = "documents" | "datasets" | "training" | "models" | "settings";

export default function ProjectDetailPage() {
  const params = useParams<{ id: string }>();
  const { data: project, isLoading, error } = useProject(params.id);
  const { data: pipelineStatus } = usePipelineStatus(params.id);
  const isActive =
    (pipelineStatus?.documents.parsing ?? 0) > 0 ||
    (pipelineStatus?.datasets.generating ?? 0) > 0 ||
    (pipelineStatus?.training_jobs?.training ?? 0) > 0;
  const { data: docsData } = useDocuments(
    params.id,
    0,
    50,
    isActive ? 3000 : false,
  );
  const { data: datasetsData } = useDatasets(params.id);
  const { data: trainingJobsData } = useTrainingJobs(params.id);
  const { data: modelsData } = useModels(params.id);
  const uploadDocs = useUploadDocuments(params.id);
  const triggerParse = useTriggerParse(params.id);
  const triggerRefine = useTriggerRefine(params.id);
  const { markStepComplete } = useOnboarding();
  const { data: catalogData } = useModelCatalog();
  const catalogModels = useMemo(
    () => catalogData?.models ?? [],
    [catalogData],
  );

  const [tab, setTab] = useState<TabKey>("documents");
  const [showTrainForm, setShowTrainForm] = useState(false);
  const [showDistillSetup, setShowDistillSetup] = useState(false);
  const [showOneClickSetup, setShowOneClickSetup] = useState(false);
  // No response field carries extraction state, so a fidelity run is only known
  // to the session that launched it — enough to follow the run in progress.
  const [scoringJobIds, setScoringJobIds] = useState<string[]>([]);

  // Track onboarding steps + toast notifications for pipeline mutations
  useEffect(() => {
    if (uploadDocs.isSuccess) {
      markStepComplete("upload_document");
      toast.success(`${uploadDocs.data.length} file(s) uploaded successfully`);
    }
  }, [uploadDocs.isSuccess, uploadDocs.data, markStepComplete]);

  useEffect(() => {
    if (uploadDocs.isError) toast.error(uploadDocs.error.message);
  }, [uploadDocs.isError, uploadDocs.error]);

  useEffect(() => {
    if (triggerParse.isSuccess) {
      markStepComplete("parse_documents");
      toast.success(
        `Parse started for ${triggerParse.data.document_count} documents`,
      );
    }
  }, [triggerParse.isSuccess, triggerParse.data, markStepComplete]);

  useEffect(() => {
    if (triggerParse.isError) toast.error(triggerParse.error.message);
  }, [triggerParse.isError, triggerParse.error]);

  useEffect(() => {
    if (triggerRefine.isSuccess) {
      markStepComplete("generate_data");
      toast.success(
        `Refine started for ${triggerRefine.data.document_count} documents`,
      );
    }
  }, [triggerRefine.isSuccess, triggerRefine.data, markStepComplete]);

  useEffect(() => {
    if (triggerRefine.isError) toast.error(triggerRefine.error.message);
  }, [triggerRefine.isError, triggerRefine.error]);

  const allDocuments = docsData?.data ?? [];
  const datasets = datasetsData?.data ?? [];
  const allTrainingJobs = trainingJobsData?.data ?? [];
  const models = modelsData?.data ?? [];
  const status = pipelineStatus;

  // Scoring happens before the student starts training, so a tracked job counts
  // as scoring until it leaves the states that precede its own GPU run.
  const scoringJobCount = allTrainingJobs.filter(
    (job) =>
      scoringJobIds.includes(job.id) &&
      ["pending", "cost_approval", "provisioning"].includes(job.status),
  ).length;

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading project...</p>
      </div>
    );
  }

  if (error || !project) {
    return (
      <div className="flex flex-col items-center justify-center gap-4 py-20">
        <p className="text-zinc-500">Project not found</p>
        <Link
          href="/projects"
          className="text-sm text-zinc-900 underline hover:no-underline dark:text-white"
        >
          Back to Projects
        </Link>
      </div>
    );
  }

  const docs = status?.documents;
  const ds = status?.datasets;
  const jobs = status?.training_jobs;
  const modelCounts = status?.models;

  const hasUploaded = (docs?.uploaded ?? 0) > 0;
  const hasParsed = (docs?.parsed ?? 0) > 0;
  const isParsing = (docs?.parsing ?? 0) > 0;
  const isGenerating = (ds?.generating ?? 0) > 0;
  const hasApprovedDatasets = (ds?.approved ?? 0) > 0;
  const isEmpty = (docs?.total ?? 0) === 0 && (ds?.total ?? 0) === 0;

  const suggestedBaseModel =
    catalogData?.suggested ?? catalogModels[0]?.model_id ?? FALLBACK_BASE_MODEL;
  const taskType = project.task_type || "question_answering";

  const distillToggle = (
    <Button
      variant="secondary"
      onClick={() => {
        setShowDistillSetup(!showDistillSetup);
        setShowOneClickSetup(false);
      }}
      title="Use a big, expensive model to teach a small one you own — with a report proving how close it got"
    >
      Distill a Larger Model
    </Button>
  );

  const oneClickToggle = (
    <Button
      variant="secondary"
      onClick={() => {
        setShowOneClickSetup(!showOneClickSetup);
        setShowDistillSetup(false);
      }}
      title="Run the entire pipeline unattended: parse → generate data → train → evaluate"
    >
      One-Click Fine-Tune
    </Button>
  );

  const activeJob = allTrainingJobs.find((job) =>
    ["training", "provisioning", "pending", "cost_approval"].includes(
      job.status,
    ),
  );
  const firstReviewPending = datasets.find(
    (d) => d.status === "review_pending",
  );

  const renderNextAction = () => {
    if (!status) return null;

    if (isParsing) {
      return (
        <NextActionCard
          progress
          title={`Parsing ${docs!.parsing} document(s)…`}
          detail="Extracting text and structure. Data generation unlocks when parsing completes."
        />
      );
    }

    if (isGenerating) {
      return (
        <NextActionCard
          progress
          title="Generating training data…"
          detail="The LLM is writing training pairs from your documents — this takes a few minutes. You'll review the dataset before anything trains."
        />
      );
    }

    if (scoringJobCount > 0) {
      return (
        <NextActionCard
          progress
          title="Scoring with the teacher model…"
          detail="The teacher scores every training example first. Student training starts automatically when scoring completes."
        >
          {activeJob && (
            <Link
              href={`/projects/${params.id}/training/${activeJob.id}`}
              className="text-sm font-medium text-violet-600 underline-offset-2 hover:underline dark:text-violet-400"
            >
              View job →
            </Link>
          )}
        </NextActionCard>
      );
    }

    if ((jobs?.training ?? 0) > 0 || (jobs?.pending ?? 0) > 0) {
      return (
        <NextActionCard
          progress
          title="Training in progress…"
          detail="Loss, GPU utilization and progress stream live on the job page."
        >
          {activeJob && (
            <Link
              href={`/projects/${params.id}/training/${activeJob.id}`}
              className="text-sm font-medium text-violet-600 underline-offset-2 hover:underline dark:text-violet-400"
            >
              View live metrics →
            </Link>
          )}
        </NextActionCard>
      );
    }

    if (activeJob?.status === "cost_approval") {
      return (
        <NextActionCard
          title="Training needs your approval"
          detail="The estimated cost exceeds the approval threshold. Approve or reject it on the job page."
        >
          <Link href={`/projects/${params.id}/training/${activeJob.id}`}>
            <Button>Review cost</Button>
          </Link>
        </NextActionCard>
      );
    }

    if (hasUploaded && !hasParsed) {
      return (
        <NextActionCard
          title="Parse your documents"
          detail={`${docs!.uploaded} document(s) uploaded. Parsing extracts the text your training data is generated from.`}
        >
          <Button
            onClick={() => triggerParse.mutate()}
            loading={triggerParse.isPending}
          >
            {triggerParse.isPending ? "Starting..." : "Parse Documents"}
          </Button>
          {oneClickToggle}
          {distillToggle}
        </NextActionCard>
      );
    }

    if (firstReviewPending) {
      return (
        <NextActionCard
          title="Review your dataset"
          detail={`${ds!.review_pending} dataset(s) awaiting review. Approve the pairs you want to train on — nothing trains without your sign-off.`}
        >
          <Link
            href={`/projects/${params.id}/dataset?datasetId=${firstReviewPending.id}`}
          >
            <Button>Review dataset</Button>
          </Link>
        </NextActionCard>
      );
    }

    if (hasParsed && !hasApprovedDatasets) {
      return (
        <NextActionCard
          title="Generate training data"
          detail={`${docs!.parsed} parsed document(s) ready. Generate question–answer pairs automatically, or steer the generation in Data Studio.`}
        >
          <Button
            onClick={() => triggerRefine.mutate({ taskType })}
            loading={triggerRefine.isPending}
          >
            {triggerRefine.isPending ? "Starting..." : "Generate Training Data"}
          </Button>
          <Link href={`/projects/${params.id}/data-studio`}>
            <Button
              variant="secondary"
              title="Review facets and preview samples before generating the dataset"
            >
              Data Studio (Guided)
            </Button>
          </Link>
          {distillToggle}
          {hasUploaded && (
            <Button
              variant="ghost"
              onClick={() => triggerParse.mutate()}
              loading={triggerParse.isPending}
              title={`${docs!.uploaded} newer document(s) are not parsed yet`}
            >
              Parse {docs!.uploaded} new document(s)
            </Button>
          )}
        </NextActionCard>
      );
    }

    if (hasApprovedDatasets && (modelCounts?.total ?? 0) === 0) {
      return (
        <NextActionCard
          title="Train your model"
          detail={`${ds!.approved} approved dataset(s) ready. Pick a base model and method — QLoRA on a small model is the fastest first run.`}
        >
          <Button
            onClick={() => {
              setTab("training");
              setShowTrainForm(true);
            }}
          >
            Start Training
          </Button>
          {distillToggle}
        </NextActionCard>
      );
    }

    if ((modelCounts?.total ?? 0) > 0) {
      return (
        <NextActionCard
          title="Model ready"
          detail={
            (modelCounts?.active ?? 0) > 0
              ? "Your model is deployed. Try it in the playground or compare it against the base model."
              : "Training produced a model. Review it, run an evaluation, or deploy it for inference."
          }
        >
          <Button onClick={() => setTab("models")}>View Models</Button>
          {(modelCounts?.active ?? 0) > 0 && (
            <Link href={`/projects/${params.id}/playground`}>
              <Button variant="secondary">Open Playground</Button>
            </Link>
          )}
          {hasApprovedDatasets && (
            <Button
              variant="ghost"
              onClick={() => {
                setTab("training");
                setShowTrainForm(true);
              }}
            >
              Train another
            </Button>
          )}
        </NextActionCard>
      );
    }

    if ((docs?.failed ?? 0) > 0) {
      return (
        <NextActionCard
          title="Parsing failed"
          detail="All documents failed to parse. Check the error on each file in the Documents tab, then upload a corrected copy."
        >
          <Button onClick={() => setTab("documents")}>View documents</Button>
        </NextActionCard>
      );
    }

    return null;
  };

  const tabs = [
    { key: "documents", label: "Documents", count: docs?.total },
    { key: "datasets", label: "Datasets", count: ds?.total },
    { key: "training", label: "Training", count: jobs?.total },
    { key: "models", label: "Models", count: modelCounts?.total },
    { key: "settings", label: "Settings" },
  ];

  return (
    <div>
      {/* Header */}
      <div className="mb-6">
        <Breadcrumbs
          items={[
            { label: "Projects", href: "/projects" },
            { label: project.name },
          ]}
        />
        <div className="flex items-center gap-3">
          <h1 className="truncate text-xl font-bold text-zinc-900 dark:text-white md:text-2xl">
            {project.name}
          </h1>
          <StatusBadge status={project.status} />
        </div>
        {project.description && (
          <p className="mt-1 text-zinc-500">{project.description}</p>
        )}
      </div>

      {/* Pipeline overview: where you are + the one next action */}
      {isEmpty ? (
        <div className="mb-8 space-y-6">
          <RouteExplainer onImport={() => setTab("datasets")} />
          <DocumentDropzone uploadDocs={uploadDocs} />
        </div>
      ) : (
        <div className="mb-8 space-y-4">
          {status && (
            <PipelineStepper
              steps={computePipelineSteps(status, scoringJobCount)}
            />
          )}
          {renderNextAction()}
          {showOneClickSetup && (
            <OneClickSetup
              projectId={params.id}
              taskType={taskType}
              catalogModels={catalogModels}
              suggestedBaseModel={suggestedBaseModel}
              onStarted={() => {
                setShowOneClickSetup(false);
                toast.success("Full pipeline started");
              }}
            />
          )}
          {showDistillSetup && (
            <DistillSetup
              projectId={params.id}
              taskType={taskType}
              catalogModels={catalogModels}
              suggestedBaseModel={suggestedBaseModel}
              disabled={!hasUploaded && !hasParsed}
              onStarted={() => setShowDistillSetup(false)}
            />
          )}
        </div>
      )}

      {/* Reference data */}
      <TabBar
        tabs={tabs}
        active={tab}
        onChange={(key) => setTab(key as TabKey)}
      />
      <div className="pt-6">
        {tab === "documents" && (
          <DocumentsTab allDocuments={allDocuments} uploadDocs={uploadDocs} />
        )}
        {tab === "datasets" && (
          <DatasetsTab
            projectId={params.id}
            datasets={datasets}
            hasParsedDocuments={hasParsed}
            onGenerate={() => triggerRefine.mutate({ taskType })}
            generatePending={triggerRefine.isPending}
          />
        )}
        {tab === "training" && (
          <TrainingTab
            projectId={params.id}
            datasets={datasets}
            allTrainingJobs={allTrainingJobs}
            showTrainForm={showTrainForm}
            setShowTrainForm={setShowTrainForm}
            onDistillJobCreated={(jobId) =>
              setScoringJobIds((prev) => [...prev, jobId])
            }
          />
        )}
        {tab === "models" && (
          <ModelsTab projectId={params.id} models={models} />
        )}
        {tab === "settings" && <SettingsTab project={project} />}
      </div>
    </div>
  );
}
