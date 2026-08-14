import * as React from "react";
import { Building2, Coins, Medal, RefreshCw, Sparkles, Trophy, Users } from "lucide-react";

import { useEconomyLeaderboardQuery } from "@/features/agents/hooks";
import type { EconomyAgentSnapshot } from "@/shared/api/tauri";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";

export type LeaderboardBoardId =
  | "overall"
  | "credits"
  | "reputation"
  | "agents"
  | "orgs"
  | "achievements";

const BOARDS: {
  id: LeaderboardBoardId;
  label: string;
  description: string;
  icon: React.ComponentType<{ className?: string }>;
}[] = [
  {
    id: "overall",
    label: "Overall",
    description: "Reputation first, then credits",
    icon: Trophy,
  },
  {
    id: "credits",
    label: "Credits",
    description: "Highest virtual balance",
    icon: Coins,
  },
  {
    id: "reputation",
    label: "Reputation",
    description: "Highest reputation score",
    icon: Medal,
  },
  {
    id: "agents",
    label: "Agents",
    description: "Individual agents only",
    icon: Users,
  },
  {
    id: "orgs",
    label: "Orgs",
    description: "Organizations only",
    icon: Building2,
  },
  {
    id: "achievements",
    label: "Achievements",
    description: "Most unlocked achievements",
    icon: Sparkles,
  },
];

const TIER_DISPLAY: Record<string, string> = {
  见习: "Novice",
  实习: "Novice",
  熟练: "Adept",
  专家: "Expert",
  大师: "Master",
  传奇: "Legend",
  Novice: "Novice",
  Adept: "Adept",
  Expert: "Expert",
  Master: "Master",
  Legend: "Legend",
};

export function displayEconomyTier(tier: string): string {
  return TIER_DISPLAY[tier] ?? tier;
}

function isOrgRow(row: EconomyAgentSnapshot): boolean {
  return row.pubkey.startsWith("org:");
}

function sortRows(
  rows: EconomyAgentSnapshot[],
  board: LeaderboardBoardId,
): EconomyAgentSnapshot[] {
  const filtered = rows.filter((row) => {
    if (board === "agents") return !isOrgRow(row);
    if (board === "orgs") return isOrgRow(row);
    return true;
  });

  const sorted = [...filtered];
  sorted.sort((a, b) => {
    switch (board) {
      case "credits":
        return (
          b.balance - a.balance ||
          b.reputation - a.reputation ||
          a.name.localeCompare(b.name)
        );
      case "reputation":
        return (
          b.reputation - a.reputation ||
          b.balance - a.balance ||
          a.name.localeCompare(b.name)
        );
      case "achievements": {
        const aCount = a.achievements?.length ?? 0;
        const bCount = b.achievements?.length ?? 0;
        return (
          bCount - aCount ||
          b.reputation - a.reputation ||
          b.balance - a.balance ||
          a.name.localeCompare(b.name)
        );
      }
      case "overall":
      case "agents":
      case "orgs":
      default:
        return (
          b.reputation - a.reputation ||
          b.balance - a.balance ||
          a.name.localeCompare(b.name)
        );
    }
  });
  return sorted;
}

function primaryMetric(
  row: EconomyAgentSnapshot,
  board: LeaderboardBoardId,
): { label: string; value: string } {
  switch (board) {
    case "credits":
      return { label: "Credits", value: formatCredits(row.balance) };
    case "reputation":
      return { label: "Rep", value: String(row.reputation) };
    case "achievements":
      return {
        label: "Unlocked",
        value: String(row.achievements?.length ?? 0),
      };
    default:
      return { label: "Rep", value: String(row.reputation) };
  }
}

function formatCredits(balance: number): string {
  return `¤${balance.toLocaleString()}`;
}

export function EconomyLeaderboardPanel() {
  const query = useEconomyLeaderboardQuery();
  const rows = query.data ?? [];
  const profilePanel = useProfilePanel();
  const [board, setBoard] = React.useState<LeaderboardBoardId>("overall");

  const ranked = React.useMemo(() => sortRows(rows, board), [board, rows]);
  const boardMeta = BOARDS.find((item) => item.id === board) ?? BOARDS[0];
  const agentCount = rows.filter((row) => !isOrgRow(row)).length;
  const orgCount = rows.filter((row) => isOrgRow(row)).length;
  const totalCredits = rows.reduce((sum, row) => sum + row.balance, 0);

  return (
    <section
      aria-label="Room economy leaderboards"
      className="flex flex-col gap-5"
      data-testid="economy-leaderboard"
    >
      <div className="grid grid-cols-3 gap-3">
        <StatCard label="Agents" value={String(agentCount)} />
        <StatCard label="Organizations" value={String(orgCount)} />
        <StatCard label="Credits in play" value={formatCredits(totalCredits)} />
      </div>

      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <Tabs
            onValueChange={(value) => setBoard(value as LeaderboardBoardId)}
            value={board}
          >
            <TabsList className="h-auto flex-wrap justify-start gap-1 bg-muted/60 p-1">
              {BOARDS.map((item) => {
                const Icon = item.icon;
                return (
                  <TabsTrigger
                    className="gap-1.5 data-[state=active]:shadow-sm"
                    key={item.id}
                    value={item.id}
                  >
                    <Icon className="h-3.5 w-3.5" />
                    {item.label}
                  </TabsTrigger>
                );
              })}
            </TabsList>
          </Tabs>
          <Button
            className="shrink-0"
            disabled={query.isFetching}
            onClick={() => {
              void query.refetch();
            }}
            size="sm"
            type="button"
            variant="outline"
          >
            <RefreshCw
              className={cn("h-3.5 w-3.5", query.isFetching && "animate-spin")}
            />
            Refresh
          </Button>
        </div>
        <p className="text-muted-foreground text-sm">{boardMeta.description}</p>
      </div>

      <div className="overflow-hidden rounded-2xl border border-border/70 bg-background shadow-sm">
        <div className="text-muted-foreground grid grid-cols-[3rem_minmax(0,1fr)_7rem_6rem_5rem] gap-3 border-b border-border/60 bg-muted/30 px-4 py-2.5 text-[11px] font-medium tracking-wide uppercase">
          <span>Rank</span>
          <span>Name</span>
          <span>Tier</span>
          <span className="text-right">Credits</span>
          <span className="text-right">
            {board === "achievements" ? "Badges" : "Rep"}
          </span>
        </div>

        {query.isError ? (
          <p className="text-destructive px-4 py-8 text-sm">
            {query.error instanceof Error
              ? query.error.message
              : "Failed to load leaderboards"}
          </p>
        ) : ranked.length === 0 ? (
          <div className="px-4 py-12 text-center">
            <p className="text-sm font-medium">No standings yet</p>
            <p className="text-muted-foreground mt-1 text-xs">
              Complete an auction or settlement to populate credits and tiers.
            </p>
          </div>
        ) : (
          <ol className="divide-y divide-border/50">
            {ranked.slice(0, 25).map((row, index) => (
              <LeaderboardRow
                board={board}
                index={index}
                key={row.pubkey || row.name}
                onOpenLedger={() => {
                  if (!row.pubkey || isOrgRow(row)) return;
                  profilePanel.openProfilePanel?.(row.pubkey, {
                    tab: "ledger",
                  });
                }}
                row={row}
              />
            ))}
          </ol>
        )}
      </div>
    </section>
  );
}

function StatCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-2xl border border-border/60 bg-muted/20 px-4 py-3">
      <p className="text-muted-foreground text-[11px] font-medium tracking-wide uppercase">
        {label}
      </p>
      <p className="mt-1 text-xl font-semibold tracking-tight tabular-nums">
        {value}
      </p>
    </div>
  );
}

function LeaderboardRow({
  board,
  index,
  onOpenLedger,
  row,
}: {
  board: LeaderboardBoardId;
  index: number;
  onOpenLedger?: () => void;
  row: EconomyAgentSnapshot;
}) {
  const org = isOrgRow(row);
  const clickable = !org && Boolean(row.pubkey);
  const metric = primaryMetric(row, board);
  const achievementCount = row.achievements?.length ?? 0;

  return (
    <li>
      <button
        className={cn(
          "grid w-full grid-cols-[3rem_minmax(0,1fr)_7rem_6rem_5rem] items-center gap-3 px-4 py-3 text-left text-sm transition-colors",
          clickable ? "hover:bg-muted/40" : "cursor-default",
          index < 3 && "bg-primary/[0.03]",
        )}
        disabled={!clickable}
        onClick={onOpenLedger}
        type="button"
      >
        <RankMark index={index} />
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate font-medium">{row.name}</span>
            {org ? (
              <Badge className="shrink-0 text-[10px]" variant="outline">
                Org
              </Badge>
            ) : null}
          </div>
          {(row.tags ?? []).length > 0 ? (
            <div className="mt-1 flex min-w-0 flex-wrap gap-1">
              {(row.tags ?? []).slice(0, 3).map((tag) => (
                <Badge className="text-[10px]" key={tag} variant="outline">
                  {tag}
                </Badge>
              ))}
            </div>
          ) : (
            <p className="text-muted-foreground mt-0.5 text-xs">
              {metric.label}: {metric.value}
            </p>
          )}
        </div>
        <div>
          <EconomyTierBadge tier={row.tier} />
        </div>
        <span className="text-right font-medium tabular-nums">
          {formatCredits(row.balance)}
        </span>
        <span className="text-muted-foreground text-right tabular-nums">
          {board === "achievements" ? achievementCount : row.reputation}
        </span>
      </button>
    </li>
  );
}

function RankMark({ index }: { index: number }) {
  if (index === 0) {
    return (
      <span className="inline-flex h-7 w-7 items-center justify-center rounded-full bg-amber-500/15 text-xs font-semibold text-amber-700 dark:text-amber-300">
        1
      </span>
    );
  }
  if (index === 1) {
    return (
      <span className="inline-flex h-7 w-7 items-center justify-center rounded-full bg-slate-400/20 text-xs font-semibold text-slate-600 dark:text-slate-300">
        2
      </span>
    );
  }
  if (index === 2) {
    return (
      <span className="inline-flex h-7 w-7 items-center justify-center rounded-full bg-orange-500/15 text-xs font-semibold text-orange-700 dark:text-orange-300">
        3
      </span>
    );
  }
  return (
    <span className="text-muted-foreground inline-flex h-7 w-7 items-center justify-center text-xs tabular-nums">
      {index + 1}
    </span>
  );
}

export function EconomyTierBadge({
  tier,
  balance,
}: {
  tier: string;
  balance?: number;
}) {
  return (
    <Badge className="gap-1 text-[10px] font-normal" variant="secondary">
      <span>{displayEconomyTier(tier)}</span>
      {typeof balance === "number" ? (
        <span className="text-muted-foreground tabular-nums">
          {formatCredits(balance)}
        </span>
      ) : null}
    </Badge>
  );
}

export function economySnapshotForPubkey(
  rows: EconomyAgentSnapshot[] | undefined,
  pubkey: string,
): EconomyAgentSnapshot | undefined {
  return rows?.find((row) => row.pubkey === pubkey);
}
