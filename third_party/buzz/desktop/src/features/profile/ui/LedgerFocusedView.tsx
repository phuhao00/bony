import {
  useEconomyAdminMutation,
  useEconomyWalletQuery,
} from "@/features/agents/hooks";
import { EconomyTierBadge } from "@/features/agents/ui/EconomyLeaderboardPanel";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import * as React from "react";

export function LedgerFocusedView({
  agentPubkey,
  agentName,
  variant = "embedded",
  canAdmin = true,
}: {
  agentPubkey: string;
  agentName?: string;
  variant?: "embedded" | "focused";
  canAdmin?: boolean;
}) {
  const wallet = useEconomyWalletQuery(agentPubkey);
  const admin = useEconomyAdminMutation();
  const [goldDelta, setGoldDelta] = React.useState("10");
  const [repDelta, setRepDelta] = React.useState("5");
  const [tagsInput, setTagsInput] = React.useState("");
  const data = wallet.data;

  React.useEffect(() => {
    if (data?.tags) {
      setTagsInput(data.tags.join(", "));
    }
  }, [data?.tags]);

  return (
    <div
      className={variant === "focused" ? "space-y-3 pt-4" : "space-y-3"}
      data-testid={`user-profile-ledger-${agentPubkey}`}
    >
      {wallet.isLoading ? (
        <p className="text-muted-foreground text-sm">Loading ledger…</p>
      ) : wallet.isError ? (
        <p className="text-destructive text-sm">
          {wallet.error instanceof Error
            ? wallet.error.message
            : "Failed to load ledger"}
        </p>
      ) : !data ? (
        <div className="rounded-2xl bg-muted/20 px-4 py-6 text-center">
          <p className="text-sm font-medium">No ledger yet</p>
          <p className="text-muted-foreground mt-1 text-xs">
            Starting balance ¤100 · Appears after auction settlement or manual
            adjustment.
          </p>
        </div>
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-3 rounded-2xl bg-muted/20 px-4 py-3">
            <EconomyTierBadge balance={data.balance} tier={data.tier} />
            <span className="text-sm tabular-nums">
              Reputation {data.reputation}
            </span>
            <span className="text-sm tabular-nums">
              Credits ¤{data.balance}
            </span>
          </div>

          <div className="space-y-2">
            <p className="text-muted-foreground text-xs font-medium">Tags</p>
            <div className="flex flex-wrap gap-1.5">
              {(data.tags ?? []).length === 0 ? (
                <span className="text-muted-foreground text-xs">None</span>
              ) : (
                (data.tags ?? []).map((tag) => (
                  <Badge key={tag} variant="outline">
                    {tag}
                  </Badge>
                ))
              )}
            </div>
          </div>

          <div className="space-y-2">
            <p className="text-muted-foreground text-xs font-medium">
              Achievements
            </p>
            <div className="flex flex-wrap gap-1.5">
              {(data.achievements ?? []).length === 0 ? (
                <span className="text-muted-foreground text-xs">None</span>
              ) : (
                (data.achievements ?? []).map((id) => (
                  <Badge key={id} variant="secondary">
                    {id}
                  </Badge>
                ))
              )}
            </div>
          </div>

          {(data.capabilityGrants ?? []).length > 0 ? (
            <div className="space-y-2">
              <p className="text-muted-foreground text-xs font-medium">
                Evolved capabilities (routing only)
              </p>
              <div className="flex flex-wrap gap-1.5">
                {(data.capabilityGrants ?? []).map((id) => (
                  <Badge key={id} variant="outline">
                    {id}
                  </Badge>
                ))}
              </div>
            </div>
          ) : null}

          <div className="space-y-2">
            <p className="text-muted-foreground text-xs font-medium">
              Recent activity
            </p>
            {(data.history ?? []).length === 0 ? (
              <p className="text-muted-foreground text-xs">No history yet</p>
            ) : (
              <ul className="overflow-hidden rounded-2xl bg-muted/20 text-xs">
                {(data.history ?? []).map((entry) => (
                  <li
                    className="flex items-start justify-between gap-3 border-b border-border/40 px-3 py-2 last:border-b-0"
                    key={`${entry.ts}-${entry.kind}-${entry.amount}-${entry.reputationDelta}-${entry.note ?? ""}`}
                  >
                    <div className="min-w-0">
                      <p className="font-medium">{entry.kind}</p>
                      <p className="text-muted-foreground truncate">
                        {entry.note ?? entry.taskRef ?? "-"}
                      </p>
                    </div>
                    <div className="shrink-0 text-right tabular-nums">
                      <p>
                        {entry.amount >= 0 ? "+" : ""}
                        {entry.amount}
                      </p>
                      <p className="text-muted-foreground">
                        rep {entry.reputationDelta >= 0 ? "+" : ""}
                        {entry.reputationDelta}
                      </p>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </div>

          {canAdmin ? (
            <div className="space-y-2 rounded-2xl border border-border/50 p-3">
              <p className="text-xs font-medium">Admin adjustments</p>
              <div className="flex flex-wrap items-end gap-2">
                <div className="flex w-24 flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">Credits Δ</span>
                  <Input
                    aria-label="Credit adjustment"
                    onChange={(e) => setGoldDelta(e.target.value)}
                    type="number"
                    value={goldDelta}
                  />
                </div>
                <Button
                  disabled={admin.adjustBalance.isPending}
                  onClick={() => {
                    const delta = Number(goldDelta);
                    if (!Number.isFinite(delta) || delta === 0) return;
                    admin.adjustBalance.mutate({
                      pubkey: agentPubkey,
                      name: agentName ?? data.name,
                      delta: Math.trunc(delta),
                    });
                  }}
                  size="sm"
                  type="button"
                >
                  Adjust credits
                </Button>
                <div className="flex w-24 flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">Reputation Δ</span>
                  <Input
                    aria-label="Reputation adjustment"
                    onChange={(e) => setRepDelta(e.target.value)}
                    type="number"
                    value={repDelta}
                  />
                </div>
                <Button
                  disabled={admin.adjustReputation.isPending}
                  onClick={() => {
                    const delta = Number(repDelta);
                    if (!Number.isFinite(delta) || delta === 0) return;
                    admin.adjustReputation.mutate({
                      pubkey: agentPubkey,
                      name: agentName ?? data.name,
                      delta: Math.trunc(delta),
                    });
                  }}
                  size="sm"
                  type="button"
                >
                  Adjust reputation
                </Button>
              </div>
              <div className="flex flex-wrap items-end gap-2">
                <div className="flex min-w-[12rem] flex-1 flex-col gap-1 text-xs">
                  <span className="text-muted-foreground">
                    Tags (comma-separated)
                  </span>
                  <Input
                    aria-label="Tags"
                    onChange={(e) => setTagsInput(e.target.value)}
                    value={tagsInput}
                  />
                </div>
                <Button
                  disabled={admin.setTags.isPending}
                  onClick={() => {
                    const tags = tagsInput
                      .split(",")
                      .map((t) => t.trim())
                      .filter(Boolean);
                    admin.setTags.mutate({
                      pubkey: agentPubkey,
                      name: agentName ?? data.name,
                      tags,
                    });
                  }}
                  size="sm"
                  type="button"
                >
                  Save tags
                </Button>
              </div>
            </div>
          ) : null}
        </>
      )}
    </div>
  );
}
