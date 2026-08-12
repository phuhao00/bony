import {
  useEconomyAdminMutation,
  useEconomyTendersQuery,
} from "@/features/agents/hooks";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import * as React from "react";

export function TenderMarketPanel() {
  const query = useEconomyTendersQuery("open");
  const admin = useEconomyAdminMutation();
  const [title, setTitle] = React.useState("");
  const [capability, setCapability] = React.useState("research.web");
  const [budget, setBudget] = React.useState("40");
  const rows = query.data ?? [];

  const onPublish = () => {
    const trimmed = title.trim();
    const cap = capability.trim();
    const budgetNum = Number(budget);
    if (!trimmed || !cap || !Number.isFinite(budgetNum) || budgetNum <= 0) {
      return;
    }
    admin.publishTender.mutate({
      title: trimmed,
      capability: cap,
      budget: Math.floor(budgetNum),
      taskRef: `ui-${Date.now()}`,
    });
    setTitle("");
  };

  return (
    <section
      aria-label="Tender market"
      className="flex flex-col gap-4"
      data-testid="economy-tender-market"
    >
      <div className="rounded-2xl border border-border/70 bg-muted/15 p-4">
        <p className="mb-3 text-sm font-medium">Publish tender</p>
        <div className="flex flex-wrap items-end gap-2">
          <div className="flex min-w-[12rem] flex-1 flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Title</span>
            <Input
              aria-label="Tender title"
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Task title"
              value={title}
            />
          </div>
          <div className="flex w-44 flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Capability</span>
            <Input
              aria-label="Tender capability"
              onChange={(e) => setCapability(e.target.value)}
              value={capability}
            />
          </div>
          <div className="flex w-28 flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Budget</span>
            <Input
              aria-label="Tender budget"
              onChange={(e) => setBudget(e.target.value)}
              type="number"
              value={budget}
            />
          </div>
          <Button
            disabled={admin.publishTender.isPending}
            onClick={onPublish}
            size="sm"
            type="button"
          >
            Publish
          </Button>
        </div>
      </div>

      {query.isError ? (
        <p className="text-destructive text-sm">
          {query.error instanceof Error
            ? query.error.message
            : "Failed to load tender market"}
        </p>
      ) : rows.length === 0 ? (
        <div className="rounded-2xl border border-dashed border-border/70 px-4 py-10 text-center">
          <p className="text-sm font-medium">No open tenders</p>
          <p className="text-muted-foreground mt-1 text-xs">
            Publish a tender above to start collecting bids.
          </p>
        </div>
      ) : (
        <ul className="flex flex-col gap-2">
          {rows.map((tender) => (
            <li
              className="rounded-2xl border border-border/60 bg-background px-4 py-3 shadow-sm"
              key={tender.tenderId}
            >
              <div className="flex items-center justify-between gap-2">
                <span className="truncate text-sm font-medium">
                  {tender.title}
                </span>
                <Badge variant="secondary">{tender.status}</Badge>
              </div>
              <div className="text-muted-foreground mt-1.5 flex flex-wrap gap-3 text-xs tabular-nums">
                <span>{tender.capability}</span>
                <span>¤{tender.budget}</span>
                <span>
                  {tender.bids.length} bid{tender.bids.length === 1 ? "" : "s"}
                </span>
              </div>
              {tender.bids.length > 0 ? (
                <ul className="mt-2 space-y-1 text-xs text-muted-foreground">
                  {tender.bids.slice(0, 4).map((bid) => (
                    <li key={`${bid.bidderPubkey}-${bid.ts}`}>
                      @{bid.bidderName} · {bid.bidderKind} · stake ¤{bid.stake}
                    </li>
                  ))}
                </ul>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
