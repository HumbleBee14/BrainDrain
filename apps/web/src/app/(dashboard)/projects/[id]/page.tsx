"use client";

import { useParams } from "next/navigation";
import Link from "next/link";
import { toast } from "sonner";
import { useEffect, useState } from "react";
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
import { useOnboarding } from "@/hooks/use-onboarding";
import { Breadcrumbs } from "@/components/breadcrumbs";
import { TabBar } from "@/components/ui/tabs";
import {
  PipelineStepper,
  computePipelineSteps,
} from "./components/pipeline-stepper";
import { RouteExplainer } from "./components/route-explainer";
import { DocumentDropzone } from "./components/document-dropzone";
import { DataTab } from "./components/data-tab";
import { NextStepBar } from "./components/next-step-bar";
import { TrainingTab } from "./components/training-tab";
import { ModelsTab } from "./components/models-tab";
import { SettingsTab } from "./components/settings-tab";

type TabKey = "data" | "training" | "models" | "settings";

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

  const [tab, setTab] = useState<TabKey>("data");
  const [showTrainForm, setShowTrainForm] = useState(false);

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
  const hasApprovedDatasets = (ds?.approved ?? 0) > 0;
  const isEmpty = (docs?.total ?? 0) === 0 && (ds?.total ?? 0) === 0;

  const taskType = project.task_type || "question_answering";

  const tabs = [
    {
      key: "data",
      label: "Data",
      count: (docs?.total ?? 0) + (ds?.total ?? 0),
    },
    { key: "training", label: "Fine-Tuning", count: jobs?.total },
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
        <h1 className="truncate text-xl font-bold text-zinc-900 dark:text-white md:text-2xl">
          {project.name}
        </h1>
        {project.description && (
          <p className="mt-1 text-zinc-500">{project.description}</p>
        )}
      </div>

      {/* Where you are in the pipeline — informational, actions live in the tabs */}
      {isEmpty ? (
        <div className="mb-8 space-y-6">
          <RouteExplainer onImport={() => setTab("data")} />
          <DocumentDropzone uploadDocs={uploadDocs} />
        </div>
      ) : (
        status && (
          <div className="mb-8 flex items-start gap-4">
            <div className="min-w-0 flex-1">
              <PipelineStepper steps={computePipelineSteps(status)} />
            </div>
            <Link
              href={`/projects/${params.id}/lineage`}
              className="shrink-0 pt-1 text-xs font-medium text-zinc-500 underline-offset-2 hover:text-zinc-900 hover:underline dark:hover:text-white"
              title="Trace every model back through its training data to the source documents"
            >
              Data Lineage →
            </Link>
          </div>
        )
      )}

      <TabBar
        tabs={tabs}
        active={tab}
        onChange={(key) => setTab(key as TabKey)}
      />
      <div className="pt-6">
        {tab === "data" && (
          <>
            <DataTab
              projectId={params.id}
              allDocuments={allDocuments}
              uploadDocs={uploadDocs}
              datasets={datasets}
              hasParsedDocuments={hasParsed}
              canParse={hasUploaded && !isParsing}
              isParsing={isParsing}
              onParse={() => triggerParse.mutate()}
              parsePending={triggerParse.isPending}
              onGenerate={() => triggerRefine.mutate({ taskType })}
              generatePending={triggerRefine.isPending}
            />
            <NextStepBar
              label="Next: Fine-Tuning"
              enabled={hasApprovedDatasets}
              hint={
                (ds?.review_pending ?? 0) > 0
                  ? "Review and approve a dataset to unlock fine-tuning"
                  : "Generate or import a dataset, then approve it, to unlock fine-tuning"
              }
              onNext={() => setTab("training")}
            />
          </>
        )}
        {tab === "training" && (
          <>
            <TrainingTab
              projectId={params.id}
              taskType={taskType}
              canDistill={hasUploaded || hasParsed}
              datasets={datasets}
              allTrainingJobs={allTrainingJobs}
              showTrainForm={showTrainForm}
              setShowTrainForm={setShowTrainForm}
            />
            <NextStepBar
              label="Next: Models"
              enabled={(modelCounts?.total ?? 0) > 0}
              hint="Complete a fine-tuning run to unlock models"
              onNext={() => setTab("models")}
            />
          </>
        )}
        {tab === "models" && (
          <ModelsTab projectId={params.id} models={models} />
        )}
        {tab === "settings" && <SettingsTab project={project} />}
      </div>
    </div>
  );
}
