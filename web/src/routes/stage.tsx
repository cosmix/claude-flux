import { useAtomValue } from "jotai/react";
import { ArrowLeftIcon, SearchXIcon } from "lucide-react";
import { Link, useParams } from "react-router";

import { EmptyState } from "@/aurora-ui/feedback/EmptyState";

import { StageHeading } from "@/components/stage-heading";
import { StageSectionGrid } from "@/components/stage-sections";
import { Skeleton } from "@/components/ui/skeleton";
import { orderedStagesAtom, selectStage, snapshotAtom } from "@/state/atoms";

/// Route `/stages/:stageId`: everything the row shows plus every remaining
/// `StageSummary` field, grouped.
export function StagePage() {
  const { stageId = "" } = useParams<{ stageId: string }>();
  const snapshot = useAtomValue(snapshotAtom);
  const ordered = useAtomValue(orderedStagesAtom);

  if (snapshot === null) return <StageSkeleton />;
  const stage = selectStage(snapshot, stageId);
  if (stage === undefined) return <NotFound id={stageId} />;
  const level = ordered.find((entry) => entry.stage.id === stageId)?.level ?? null;

  return (
    <article className="mx-auto flex w-full max-w-[1440px] flex-col gap-5 px-4 py-5 sm:px-6">
      <BackLink />
      <StageHeading stage={stage} />
      <StageSectionGrid stage={stage} level={level} wide />
    </article>
  );
}

function BackLink() {
  return (
    <Link
      to="/"
      className="inline-flex w-fit items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground"
    >
      <ArrowLeftIcon className="size-4" />
      overview
    </Link>
  );
}

export function NotFound({ id }: { id: string }) {
  return (
    <div className="mx-auto flex w-full max-w-[1440px] flex-col gap-3 px-4 py-5 sm:px-6">
      <BackLink />
      <EmptyState
        icon={SearchXIcon}
        title="No such stage"
        description={
          <>
            No stage named <code className="rounded bg-muted px-1 py-0.5 text-xs">{id}</code> in
            this workspace.
          </>
        }
        tone="muted"
      />
    </div>
  );
}

function StageSkeleton() {
  return (
    <div
      className="mx-auto flex w-full max-w-[1440px] flex-col gap-4 px-4 py-5 sm:px-6"
      aria-busy="true"
    >
      <Skeleton className="h-4 w-16" />
      <Skeleton className="h-7 w-64" />
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        <Skeleton className="h-36" />
        <Skeleton className="h-36" />
        <Skeleton className="h-36" />
      </div>
    </div>
  );
}
