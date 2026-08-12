import {
  useEconomyAdminMutation,
  useEconomyWalletQuery,
} from "@/features/agents/hooks";
import {
  displayEconomyTier,
  EconomyTierBadge,
} from "@/features/agents/ui/EconomyLeaderboardPanel";
import type { EconomyWalletView } from "@/shared/api/tauri";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import {
  ChevronDown,
  Coins,
  Lock,
  ScrollText,
  Sparkles,
  Star,
  Swords,
  Trophy,
} from "lucide-react";
import * as React from "react";

const TIER_LADDER = [
  { name: "Novice", min: 0, next: 100 },
  { name: "Adept", min: 100, next: 500 },
  { name: "Expert", min: 500, next: 2000 },
  { name: "Master", min: 2000, next: 5000 },
  { name: "Legend", min: 5000, next: null },
] as const;

const TROPHY_CATALOG = [
  {
    id: "first_contract_won",
    title: "First Contract",
    hint: "Win and settle your first job",
  },
  {
    id: "gold_1000",
    title: "Thousand Credits",
    hint: "Hold ¤1,000 at once",
  },
  {
    id: "tier_reached_adept",
    title: "Reach Adept",
    hint: "Climb to 100 reputation",
  },
  {
    id: "tier_reached_expert",
    title: "Reach Expert",
    hint: "Climb to 500 reputation",
  },
] as const;

function tierProgress(reputation: number): {
  tier: string;
  nextTier: string | null;
  into: number;
  span: number;
  percent: number;
  remaining: number | null;
} {
  const rep = Math.max(0, reputation);
  for (let i = 0; i < TIER_LADDER.length; i += 1) {
    const step = TIER_LADDER[i];
    const next = step.next;
    if (next === null || rep < next) {
      if (next === null) {
        return {
          tier: step.name,
          nextTier: null,
          into: 1,
          span: 1,
          percent: 100,
          remaining: null,
        };
      }
      const into = rep - step.min;
      const span = next - step.min;
      return {
        tier: step.name,
        nextTier: TIER_LADDER[i + 1]?.name ?? null,
        into,
        span,
        percent: Math.min(100, Math.round((into / span) * 100)),
        remaining: next - rep,
      };
    }
  }
  return {
    tier: "Legend",
    nextTier: null,
    into: 1,
    span: 1,
    percent: 100,
    remaining: null,
  };
}

function starterSheet(name: string, pubkey: string): EconomyWalletView {
  return {
    name,
    pubkey,
    balance: 100,
    reputation: 0,
    tier: "Novice",
    tags: [],
    achievements: [],
    capabilityGrants: [],
    history: [],
  };
}

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
  const [gmOpen, setGmOpen] = React.useState(false);
  const onChain = Boolean(wallet.data);
  const data =
    wallet.data ??
    starterSheet(agentName?.trim() || "Agent", agentPubkey);
  const progress = tierProgress(data.reputation);
  const unlocked = new Set(data.achievements ?? []);

  React.useEffect(() => {
    if (wallet.data?.tags) {
      setTagsInput(wallet.data.tags.join(", "));
    }
  }, [wallet.data?.tags]);

  const awaken = () => {
    admin.adjustBalance.mutate({
      pubkey: agentPubkey,
      name: agentName ?? data.name,
      delta: 0,
      note: "character awaken — claim starter purse",
    });
  };

  return (
    <div
      className={cn(
        "space-y-3",
        variant === "focused" ? "pt-4" : undefined,
      )}
      data-testid={`user-profile-ledger-${agentPubkey}`}
    >
      {wallet.isLoading ? (
        <p className="text-muted-foreground text-sm">Loading character…</p>
      ) : wallet.isError ? (
        <p className="text-destructive text-sm">
          {wallet.error instanceof Error
            ? wallet.error.message
            : "Failed to load character sheet"}
        </p>
      ) : (
        <>
          <section className="overflow-hidden rounded-2xl border border-border/70 bg-gradient-to-b from-amber-500/[0.07] via-background to-background shadow-sm">
            <div className="flex items-center justify-between gap-2 border-b border-border/50 px-4 py-2.5">
              <div className="flex items-center gap-2">
                <Swords className="text-amber-700/80 h-4 w-4 dark:text-amber-300/80" />
                <p className="text-sm font-semibold tracking-tight">
                  Character sheet
                </p>
              </div>
              <Badge
                className="text-[10px] font-medium"
                variant={onChain ? "secondary" : "outline"}
              >
                {onChain ? "Active" : "Dormant"}
              </Badge>
            </div>

            <div className="space-y-4 px-4 py-4">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0 space-y-1">
                  <p className="text-muted-foreground text-[11px] font-medium tracking-wide uppercase">
                    Rank
                  </p>
                  <div className="flex items-center gap-2">
                    <EconomyTierBadge tier={data.tier} />
                    <span className="text-muted-foreground text-xs">
                      {displayEconomyTier(progress.tier)}
                      {progress.nextTier
                        ? ` → ${displayEconomyTier(progress.nextTier)}`
                        : " · max rank"}
                    </span>
                  </div>
                </div>
                {!onChain && canAdmin ? (
                  <Button
                    disabled={admin.adjustBalance.isPending}
                    onClick={awaken}
                    size="sm"
                    type="button"
                  >
                    <Sparkles className="h-3.5 w-3.5" />
                    Awaken character
                  </Button>
                ) : null}
              </div>

              <div className="space-y-1.5">
                <div className="flex items-center justify-between text-[11px]">
                  <span className="text-muted-foreground font-medium">
                    Reputation XP
                  </span>
                  <span className="tabular-nums">
                    {progress.nextTier
                      ? `${progress.into}/${progress.span} · ${progress.remaining} to next`
                      : `${data.reputation} XP`}
                  </span>
                </div>
                <div className="bg-muted h-2.5 overflow-hidden rounded-full">
                  <div
                    className="h-full rounded-full bg-gradient-to-r from-amber-500/80 to-orange-500/80 transition-[width] duration-500"
                    style={{ width: `${progress.percent}%` }}
                  />
                </div>
              </div>

              <div className="grid grid-cols-2 gap-2">
                <StatTile
                  icon={<Coins className="h-4 w-4 text-amber-600" />}
                  label="Credits"
                  value={`¤${data.balance.toLocaleString()}`}
                />
                <StatTile
                  icon={<Star className="h-4 w-4 text-sky-600" />}
                  label="Reputation"
                  value={String(data.reputation)}
                />
              </div>

              {!onChain ? (
                <p className="text-muted-foreground text-xs leading-relaxed">
                  Starter purse preview (¤100 · Novice). Awaken to write this
                  character onto the room ledger, or wait for the first auction
                  settlement.
                </p>
              ) : null}
            </div>
          </section>

          <SheetBlock
            icon={<ScrollText className="h-3.5 w-3.5" />}
            title="Titles"
          >
            {(data.tags ?? []).length === 0 ? (
              <p className="text-muted-foreground text-xs">
                No titles yet — earn tags from contracts and GM awards.
              </p>
            ) : (
              <div className="flex flex-wrap gap-1.5">
                {(data.tags ?? []).map((tag) => (
                  <Badge
                    className="border-amber-500/30 bg-amber-500/10 text-amber-900 dark:text-amber-100"
                    key={tag}
                    variant="outline"
                  >
                    {tag}
                  </Badge>
                ))}
              </div>
            )}
          </SheetBlock>

          <SheetBlock
            icon={<Trophy className="h-3.5 w-3.5" />}
            title="Trophy case"
          >
            <div className="grid grid-cols-2 gap-2">
              {TROPHY_CATALOG.map((trophy) => {
                const earned = unlocked.has(trophy.id);
                return (
                  <div
                    className={cn(
                      "rounded-xl border px-3 py-2.5",
                      earned
                        ? "border-amber-500/40 bg-amber-500/[0.08]"
                        : "border-border/60 bg-muted/20 opacity-80",
                    )}
                    key={trophy.id}
                  >
                    <div className="flex items-start gap-2">
                      {earned ? (
                        <Trophy className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-600" />
                      ) : (
                        <Lock className="text-muted-foreground mt-0.5 h-3.5 w-3.5 shrink-0" />
                      )}
                      <div className="min-w-0">
                        <p className="truncate text-xs font-semibold">
                          {trophy.title}
                        </p>
                        <p className="text-muted-foreground mt-0.5 text-[11px] leading-snug">
                          {earned ? "Unlocked" : trophy.hint}
                        </p>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
            {(data.achievements ?? []).some(
              (id) => !TROPHY_CATALOG.some((t) => t.id === id),
            ) ? (
              <div className="mt-2 flex flex-wrap gap-1.5">
                {(data.achievements ?? [])
                  .filter((id) => !TROPHY_CATALOG.some((t) => t.id === id))
                  .map((id) => (
                    <Badge key={id} variant="secondary">
                      {id}
                    </Badge>
                  ))}
              </div>
            ) : null}
          </SheetBlock>

          <SheetBlock
            icon={<Sparkles className="h-3.5 w-3.5" />}
            title="Skill slots"
            subtitle="Routing labels only — not ACP permissions"
          >
            {(data.capabilityGrants ?? []).length === 0 ? (
              <p className="text-muted-foreground text-xs">
                Empty. Capabilities unlock as this agent evolves on successful
                work.
              </p>
            ) : (
              <div className="flex flex-wrap gap-1.5">
                {(data.capabilityGrants ?? []).map((id) => (
                  <Badge key={id} variant="outline">
                    {id}
                  </Badge>
                ))}
              </div>
            )}
          </SheetBlock>

          <SheetBlock
            icon={<ScrollText className="h-3.5 w-3.5" />}
            title="Quest log"
          >
            {(data.history ?? []).length === 0 ? (
              <div className="rounded-xl border border-dashed border-border/70 px-3 py-4 text-center">
                <p className="text-xs font-medium">No quests logged</p>
                <p className="text-muted-foreground mt-1 text-[11px]">
                  Settlements, tenders, and awards will appear here as a feed.
                </p>
              </div>
            ) : (
              <ul className="overflow-hidden rounded-xl border border-border/50 bg-muted/15 text-xs">
                {(data.history ?? []).map((entry) => (
                  <li
                    className="flex items-start justify-between gap-3 border-b border-border/40 px-3 py-2.5 last:border-b-0"
                    key={`${entry.ts}-${entry.kind}-${entry.amount}-${entry.reputationDelta}-${entry.note ?? ""}`}
                  >
                    <div className="min-w-0">
                      <p className="font-medium capitalize">
                        {entry.kind.replaceAll("_", " ")}
                      </p>
                      <p className="text-muted-foreground truncate">
                        {entry.note ?? entry.taskRef ?? "—"}
                      </p>
                    </div>
                    <div className="shrink-0 text-right tabular-nums">
                      <p
                        className={
                          entry.amount >= 0
                            ? "text-emerald-700 dark:text-emerald-400"
                            : "text-destructive"
                        }
                      >
                        {entry.amount >= 0 ? "+" : ""}
                        {entry.amount}¤
                      </p>
                      <p className="text-muted-foreground">
                        {entry.reputationDelta >= 0 ? "+" : ""}
                        {entry.reputationDelta} rep
                      </p>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </SheetBlock>

          {canAdmin ? (
            <div className="rounded-2xl border border-border/60">
              <button
                className="flex w-full items-center justify-between gap-2 px-4 py-2.5 text-left text-xs font-medium"
                onClick={() => setGmOpen((open) => !open)}
                type="button"
              >
                <span>GM console</span>
                <ChevronDown
                  className={cn(
                    "text-muted-foreground h-3.5 w-3.5 transition-transform",
                    gmOpen && "rotate-180",
                  )}
                />
              </button>
              {gmOpen ? (
                <div className="space-y-2 border-t border-border/50 px-4 py-3">
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
                      Grant credits
                    </Button>
                    <div className="flex w-24 flex-col gap-1 text-xs">
                      <span className="text-muted-foreground">Rep Δ</span>
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
                      Grant reputation
                    </Button>
                  </div>
                  <div className="flex flex-wrap items-end gap-2">
                    <div className="flex min-w-[12rem] flex-1 flex-col gap-1 text-xs">
                      <span className="text-muted-foreground">
                        Titles (comma-separated)
                      </span>
                      <Input
                        aria-label="Titles"
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
                      Save titles
                    </Button>
                  </div>
                </div>
              ) : null}
            </div>
          ) : null}
        </>
      )}
    </div>
  );
}

function StatTile({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-xl border border-border/60 bg-background/80 px-3 py-2.5">
      <div className="text-muted-foreground flex items-center gap-1.5 text-[11px] font-medium tracking-wide uppercase">
        {icon}
        {label}
      </div>
      <p className="mt-1 text-lg font-semibold tracking-tight tabular-nums">
        {value}
      </p>
    </div>
  );
}

function SheetBlock({
  children,
  icon,
  subtitle,
  title,
}: {
  children: React.ReactNode;
  icon: React.ReactNode;
  subtitle?: string;
  title: string;
}) {
  return (
    <section className="rounded-2xl border border-border/60 bg-muted/10 px-4 py-3">
      <div className="mb-2.5 flex items-baseline justify-between gap-2">
        <div className="flex items-center gap-1.5">
          <span className="text-muted-foreground">{icon}</span>
          <h3 className="text-xs font-semibold tracking-wide uppercase">
            {title}
          </h3>
        </div>
        {subtitle ? (
          <span className="text-muted-foreground text-[10px]">{subtitle}</span>
        ) : null}
      </div>
      {children}
    </section>
  );
}
