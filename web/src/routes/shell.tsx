import { cn } from "cn";
import { useAtomValue } from "jotai/react";
import { useEffect, useState } from "react";
import { Outlet, useLocation } from "react-router";

import { ErrorBoundary } from "@/aurora-ui/feedback/ErrorBoundary";

import { Header } from "@/components/header";
import { useNow } from "@/components/hooks/use-now";
import { LegendDialog } from "@/components/legend-dialog";
import { QuotaMeters } from "@/components/quota-meters";
import { SettingsDialog } from "@/components/settings-dialog";
import { toneClass } from "@/components/state-badge";
import { StageModal } from "@/components/stage-modal";
import { Kbd } from "@/components/ui/kbd";
import { TooltipProvider } from "@/components/ui/tooltip";
import { formatClock, formatElapsed } from "@/lib/format";
import { providerRows } from "@/lib/quota";
import { snapshotAtom } from "@/state/atoms";

/// Header, routed body, footer, the stage dialog (`?stage=<id>` on any
/// route), the settings dialog (`?settings=<scope>`), and the legend dialog
/// with its `?` shortcut. Each route sets its
/// own width: the overview runs wide, the ledger and stage pages are bounded.
export function Shell() {
  const [legendOpen, setLegendOpen] = useState(false);
  const { pathname } = useLocation();

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "?" || event.altKey || event.ctrlKey || event.metaKey) return;
      if (isTyping(event.target)) return;
      event.preventDefault();
      setLegendOpen((open) => !open);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  return (
    <TooltipProvider delayDuration={250}>
      <div className="flex h-dvh flex-col">
        <Header onOpenLegend={() => setLegendOpen(true)} />
        <main className="flex min-h-0 w-full flex-1 flex-col overflow-y-auto">
          <ErrorBoundary resetKey={pathname}>
            <Outlet />
          </ErrorBoundary>
        </main>
        <Footer />
        <StageModal />
        <SettingsDialog />
        <LegendDialog open={legendOpen} onOpenChange={setLegendOpen} />
      </div>
    </TooltipProvider>
  );
}

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

/// Sticky status bar: provider quota gauges on the left, the legend hint and
/// the snapshot's source and time on the right. Without quota data it is the
/// plain one-line footer with the hint on the left.
function Footer() {
  const snapshot = useAtomValue(snapshotAtom);
  const now = useNow();
  const nowSecs = Math.floor(now / 1000);
  const quota = snapshot?.status.quota ?? null;
  const hasQuota = quota !== null && providerRows(quota).length > 0;
  const ageSecs = snapshot
    ? Math.max(0, Math.floor((now - Date.parse(snapshot.generated_at)) / 1000))
    : 0;
  const degraded = snapshot?.source === "files";
  return (
    <footer className="z-10 border-t border-border bg-background/90 backdrop-blur-sm">
      <div className="mx-auto flex w-full max-w-[1440px] flex-wrap items-center gap-x-4 gap-y-1 px-4 py-3 text-xs text-muted-foreground sm:px-6">
        {quota && (
          <QuotaMeters snapshot={quota} nowSecs={nowSecs} className="max-[899px]:basis-full" />
        )}
        <span
          className={cn(
            "inline-flex items-center gap-4",
            hasQuota ? "ml-auto" : "flex-1 justify-between",
          )}
        >
          <span className="inline-flex items-center gap-1.5">
            <Kbd>?</Kbd> legend
          </span>
          {snapshot && (
            <span className="font-mono tabular-nums">
              updated {formatClock(snapshot.generated_at)} · {formatElapsed(ageSecs)} ago · via{" "}
              <span className={cn(degraded && toneClass("warning"))}>{snapshot.source}</span>
            </span>
          )}
        </span>
      </div>
    </footer>
  );
}
