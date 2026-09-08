import { useAtomValue } from "jotai/react";
import { Link } from "react-router";
import { CircleHelpIcon, SlidersHorizontalIcon } from "lucide-react";

import { ThemeToggle } from "@/aurora-ui/theme/ThemeToggle";
import { WorkRoundel } from "@/components/activity-roundel";
import { ConnectionBadge } from "@/components/connection-badge";
import { DaemonLine, MergeLine, ProgressLine, SummaryLine } from "@/components/header-lines";
import { Logo } from "@/components/logo";
import { useOpenSettings } from "@/components/settings-dialog";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Skeleton } from "@/components/ui/skeleton";
import { ViewSwitch } from "@/components/view-switch";
import { attentionAtom, snapshotAtom } from "@/state/atoms";

/// The TUI's header block: the logo on the left spanning four lines, the plan
/// name and loom version, progress, counts and merge lines beside it, daemon
/// and socket state on the right.
export function Header({ onOpenLegend }: { onOpenLegend: () => void }) {
  const snapshot = useAtomValue(snapshotAtom);
  const attention = useAtomValue(attentionAtom);
  const openSettings = useOpenSettings();

  return (
    <header className="border-b border-hairline bg-linear-to-b from-card to-background">
      <div className="mx-auto grid max-w-[1920px] grid-cols-[auto_1fr] gap-x-5 px-4 py-4 sm:px-6">
        <Link to="/" className="row-span-2 self-start pt-1 text-foreground" aria-label="overview">
          <Logo className="h-10 w-auto sm:h-12" />
        </Link>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
          <PlanName name={snapshot?.status.plan_name} loading={snapshot === null} />
          {snapshot && (
            <span className="font-mono text-xs text-muted-foreground" title="loom version">
              v{snapshot.version}
            </span>
          )}
          <div className="ml-auto flex items-center gap-2">
            {snapshot && <WorkRoundel />}
            {snapshot && <DaemonLine snapshot={snapshot} />}
            <ConnectionBadge />
            <ViewSwitch />
            <ThemeToggle />
            <Button
              variant="ghost"
              size="sm"
              onClick={() => openSettings()}
              aria-label="open settings"
            >
              <SlidersHorizontalIcon />
              <span className="hidden sm:inline">settings</span>
            </Button>
            <Button variant="ghost" size="sm" onClick={onOpenLegend} aria-label="open legend">
              <CircleHelpIcon />
              <span className="hidden sm:inline">legend</span>
              <Kbd className="hidden sm:inline-flex">?</Kbd>
            </Button>
          </div>
        </div>
        {snapshot ? (
          <div className="mt-2 flex flex-col gap-1.5">
            <ProgressLine snapshot={snapshot} />
            <SummaryLine snapshot={snapshot} attention={attention.length} />
            <MergeLine snapshot={snapshot} />
          </div>
        ) : (
          <HeaderSkeleton />
        )}
      </div>
    </header>
  );
}

function PlanName({ name, loading }: { name: string | null | undefined; loading: boolean }) {
  if (loading) return <Skeleton className="h-6 w-48" />;
  if (!name) return <span className="text-lg text-muted-foreground">(no plan name)</span>;
  return <h1 className="text-lg font-semibold tracking-tight">{name}</h1>;
}

function HeaderSkeleton() {
  return (
    <div className="mt-2 flex flex-col gap-2" aria-busy="true">
      <Skeleton className="h-4 w-72 max-w-full" />
      <Skeleton className="h-4 w-96 max-w-full" />
      <Skeleton className="h-3 w-56 max-w-full" />
    </div>
  );
}
