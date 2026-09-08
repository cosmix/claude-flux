import { useAtomValue } from "jotai/react";
import { ArrowLeftIcon } from "lucide-react";
import { Link, useParams } from "react-router";

import { stageHref } from "@/components/stage-href";
import { TerminalView } from "@/components/terminal/terminal-view";
import type { EmulatorFactory, TerminalDeps } from "@/components/terminal/use-terminal";
import { Skeleton } from "@/components/ui/skeleton";
import { NotFound } from "@/routes/stage";
import { selectStage, snapshotAtom } from "@/state/atoms";

/// Route `/terminal/:stageId`: the stage dialog's terminal view on a page of
/// its own, for a tab that stays open beside the dashboard.
export interface TerminalPageProps {
  factory?: EmulatorFactory;
  deps?: TerminalDeps;
}

export function TerminalPage({ factory, deps }: TerminalPageProps = {}) {
  const { stageId = "" } = useParams<{ stageId: string }>();
  const snapshot = useAtomValue(snapshotAtom);

  if (snapshot === null) return <TerminalSkeleton />;
  const stage = selectStage(snapshot, stageId);
  if (stage === undefined) return <NotFound id={stageId} />;

  return (
    <article className="mx-auto flex min-h-0 w-full max-w-[1440px] flex-1 flex-col gap-3 px-4 py-5 sm:px-6">
      <Link
        to={stageHref(stage.id)}
        className="inline-flex w-fit items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeftIcon className="size-4" />
        stage
      </Link>
      <div className="min-h-0 flex-1 overflow-hidden rounded-xl bg-popover text-sm ring-1 ring-foreground/10">
        <TerminalView stage={stage} frame="page" factory={factory} deps={deps} />
      </div>
    </article>
  );
}

function TerminalSkeleton() {
  return (
    <div
      className="mx-auto flex min-h-0 w-full max-w-[1440px] flex-1 flex-col gap-3 px-4 py-5 sm:px-6"
      aria-busy="true"
    >
      <Skeleton className="h-4 w-16" />
      <Skeleton className="min-h-0 flex-1 rounded-xl" />
    </div>
  );
}
