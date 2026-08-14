import {
  useEconomyAdminMutation,
  useEconomyTendersQuery,
} from "@/features/agents/hooks";
import { suggestEconomyTender, sweepEconomyTenders } from "@/shared/api/tauri";
import type {
  AllocationDecision,
  TenderSnapshot,
  TenderSuggestion,
} from "@/shared/api/tauri";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { economyTendersQueryKey } from "@/features/agents/hooks";

export function TenderMarketPanel() {
  const queryClient = useQueryClient();
  const query = useEconomyTendersQuery();
  const admin = useEconomyAdminMutation();
  const [title, setTitle] = React.useState("");
  const [suggestion, setSuggestion] = React.useState<TenderSuggestion | null>(
    null,
  );
  const [latestResult, setLatestResult] = React.useState<TenderSnapshot | null>(
    null,
  );
  const [actionError, setActionError] = React.useState<string | null>(null);
  const rows = query.data ?? [];
  const results = rows.filter((row) => row.status === "resolved");
  const stuckOpen = rows.filter((row) => row.status === "open");
  const stuckResolved = results.filter(
    (row) => !(row.outcome && row.outcome.trim()),
  );
  const busy =
    admin.publishTender.isPending ||
    admin.inviteTenderBids.isPending ||
    admin.resolveTender.isPending ||
    admin.sweepTenders.isPending ||
    admin.cancelTender.isPending ||
    admin.clearTenders.isPending;

  React.useEffect(() => {
    let cancelled = false;
    void sweepEconomyTenders()
      .then(async (finished) => {
        if (cancelled) return;
        if (finished[0]) setLatestResult(finished[0]);
        await queryClient.invalidateQueries({ queryKey: economyTendersQueryKey });
      })
      .catch(() => {
        /* best-effort cleanup of leftover open tenders */
      });
    return () => {
      cancelled = true;
    };
  }, [queryClient]);

  React.useEffect(() => {
    const trimmed = title.trim();
    if (!trimmed) {
      setSuggestion(null);
      return;
    }
    let cancelled = false;
    const handle = window.setTimeout(() => {
      void suggestEconomyTender(trimmed)
        .then((next) => {
          if (!cancelled) setSuggestion(next);
        })
        .catch(() => {
          if (!cancelled) setSuggestion(null);
        });
    }, 200);
    return () => {
      cancelled = true;
      window.clearTimeout(handle);
    };
  }, [title]);

  const onPublish = () => {
    const trimmed = title.trim();
    if (!trimmed) {
      return;
    }
    setActionError(null);
    admin.publishTender.mutate(
      {
        title: trimmed,
        taskRef: `ui-${Date.now()}`,
      },
      {
        onError: (error) => {
          setActionError(
            error instanceof Error ? error.message : "发布失败",
          );
        },
        onSuccess: (tender) => {
          setTitle("");
          setSuggestion(null);
          setLatestResult(tender);
        },
      },
    );
  };

  const onCancel = (tenderId: string) => {
    setActionError(null);
    admin.cancelTender.mutate(tenderId, {
      onError: (error) => {
        setActionError(error instanceof Error ? error.message : "清理失败");
      },
      onSuccess: (tender) => {
        if (latestResult?.tenderId === tender.tenderId) {
          setLatestResult(null);
        }
      },
    });
  };

  const onClear = (mode: "stuck" | "history" | "all") => {
    setActionError(null);
    admin.clearTenders.mutate(mode, {
      onError: (error) => {
        setActionError(error instanceof Error ? error.message : "批量清理失败");
      },
      onSuccess: () => {
        setLatestResult(null);
      },
    });
  };

  return (
    <section
      aria-label="Tender market"
      className="flex flex-col gap-4"
      data-testid="economy-tender-market"
    >
      <div className="rounded-2xl border border-border/70 bg-muted/15 p-4">
        <p className="mb-1 text-sm font-medium">发一个任务</p>
        <p className="text-muted-foreground mb-3 text-xs">
          只需填写标题。各 Agent 按历史结算与声望自动报价，系统综合报价 / 声望 /
          能力做分配决策；完成后按标准评级结算并展示奖励。
        </p>
        <div className="flex flex-wrap items-end gap-2">
          <div className="flex min-w-[12rem] flex-1 flex-col gap-1 text-xs">
            <span className="text-muted-foreground">任务标题</span>
            <Input
              aria-label="Tender title"
              onChange={(e) => setTitle(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") onPublish();
              }}
              placeholder="例如：查一下今天天气 / 做一份 PDF / 1+1=?"
              value={title}
            />
          </div>
          <Button
            disabled={busy || !title.trim()}
            onClick={onPublish}
            size="sm"
            type="button"
          >
            {admin.publishTender.isPending ? "执行中…" : "发布并执行"}
          </Button>
        </div>
        {suggestion ? (
          <div className="mt-3 space-y-2" data-testid="tender-suggestion-preview">
            <p className="text-muted-foreground text-xs tabular-nums">
              开放市场 · 预算 ¤{suggestion.budget}（Agent
              将按历史均价与声望自动报价）
            </p>
            {(suggestion.tags ?? []).length > 0 ? (
              <div className="flex flex-wrap gap-1">
                {suggestion.tags.map((tag) => (
                  <Badge key={tag} variant="outline">
                    {tag}
                  </Badge>
                ))}
              </div>
            ) : null}
          </div>
        ) : null}
      </div>

      {actionError ? (
        <p className="text-destructive text-sm">{actionError}</p>
      ) : null}

      {latestResult?.status === "resolved" ? (
        <ResultCard
          emphasis
          onCancel={onCancel}
          tender={latestResult}
          title="最新结果"
        />
      ) : null}

      {query.isError ? (
        <p className="text-destructive text-sm">
          {query.error instanceof Error
            ? query.error.message
            : "加载招标结果失败"}
        </p>
      ) : null}

      <div className="space-y-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-sm font-medium">历史结果</p>
          <div className="flex flex-wrap gap-2">
            {stuckResolved.length > 0 || stuckOpen.length > 0 ? (
              <Button
                disabled={busy}
                onClick={() => onClear("stuck")}
                size="sm"
                type="button"
                variant="outline"
              >
                清理卡住的
              </Button>
            ) : null}
            {results.length > 0 ? (
              <Button
                disabled={busy}
                onClick={() => onClear("history")}
                size="sm"
                type="button"
                variant="outline"
              >
                清空历史
              </Button>
            ) : null}
          </div>
        </div>
        {results.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-border/70 px-4 py-8 text-center">
            <p className="text-sm font-medium">还没有结算结果</p>
            <p className="text-muted-foreground mt-1 text-xs">
              输入标题后点「发布并执行」，任务答案会出现在这里。
            </p>
          </div>
        ) : (
          <ul className="flex flex-col gap-2">
            {results
              .filter((row) => row.tenderId !== latestResult?.tenderId)
              .map((tender) => (
                <li key={tender.tenderId}>
                  <ResultCard onCancel={onCancel} tender={tender} />
                </li>
              ))}
          </ul>
        )}
      </div>

      {stuckOpen.length > 0 ? (
        <div className="space-y-2">
          <div className="flex items-center justify-between gap-2">
            <p className="text-sm font-medium">未匹配到 Agent 的任务</p>
            <div className="flex gap-2">
              <Button
                disabled={busy}
                onClick={() => {
                  setActionError(null);
                  admin.sweepTenders.mutate(undefined, {
                    onError: (error) => {
                      setActionError(
                        error instanceof Error ? error.message : "自动结算失败",
                      );
                    },
                    onSuccess: (finished) => {
                      if (finished[0]) setLatestResult(finished[0]);
                    },
                  });
                }}
                size="sm"
                type="button"
                variant="outline"
              >
                再试一次自动结算
              </Button>
              <Button
                disabled={busy}
                onClick={() => onClear("stuck")}
                size="sm"
                type="button"
                variant="ghost"
              >
                清理
              </Button>
            </div>
          </div>
          <ul className="flex flex-col gap-2">
            {stuckOpen.map((tender) => (
              <li
                className="rounded-2xl border border-border/60 bg-background px-4 py-3 text-sm shadow-sm"
                key={tender.tenderId}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium">{tender.title}</span>
                  <div className="flex items-center gap-2">
                    <Badge variant="secondary">等待匹配</Badge>
                    <Button
                      disabled={busy}
                      onClick={() => onCancel(tender.tenderId)}
                      size="sm"
                      type="button"
                      variant="ghost"
                    >
                      删除
                    </Button>
                  </div>
                </div>
                <p className="text-muted-foreground mt-1 text-xs">
                  需要能力 {tender.capability} · 预算 ¤{tender.budget}
                  （当前没有声明该能力的 Agent）
                </p>
                <TagRow tags={tender.tags} />
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </section>
  );
}

function TagRow({ tags }: { tags?: string[] | null }) {
  const list = (tags ?? []).filter((t) => t.trim());
  if (list.length === 0) return null;
  return (
    <div className="mt-2 flex flex-wrap gap-1">
      {list.map((tag) => (
        <Badge key={tag} variant="outline">
          {tag}
        </Badge>
      ))}
    </div>
  );
}

function AllocationBoard({
  allocation,
}: {
  allocation?: AllocationDecision | null;
}) {
  if (!allocation || allocation.bids.length === 0) return null;
  return (
    <div
      className="mt-3 rounded-xl border border-border/50 bg-muted/20 px-3 py-2"
      data-testid="tender-allocation-board"
    >
      <p className="text-muted-foreground mb-1.5 text-[11px] font-medium tracking-wide uppercase">
        报价与分配决策
      </p>
      <p className="text-muted-foreground mb-2 text-xs">{allocation.note}</p>
      <ul className="flex flex-col gap-1.5">
        {allocation.bids.map((bid) => (
          <li
            className={
              bid.won
                ? "flex flex-wrap items-baseline justify-between gap-2 rounded-lg bg-primary/10 px-2 py-1.5 text-xs"
                : "flex flex-wrap items-baseline justify-between gap-2 px-2 py-1 text-xs"
            }
            key={bid.bidderPubkey}
          >
            <span className="font-medium">
              @{bid.bidderName}
              {bid.won ? " · 中标" : ""}
            </span>
            <span className="text-muted-foreground tabular-nums">
              报价 ¤{bid.quote} · 声望 {bid.reputation} · 得分{" "}
              {bid.score.toFixed(2)}
            </span>
            <span className="text-muted-foreground w-full text-[11px]">
              {bid.reason}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function ResultCard({
  tender,
  title,
  emphasis = false,
  onCancel,
}: {
  tender: TenderSnapshot;
  title?: string;
  emphasis?: boolean;
  onCancel?: (tenderId: string) => void;
}) {
  const winner = tender.winnerName?.trim() || "未知";
  const outcome = tender.outcome?.trim() || "";
  const gold = tender.rewardGold;
  const rep = tender.rewardReputation;
  const tier = tender.rewardTier?.trim() || "";
  const grade = tender.rewardGrade?.trim() || "";
  const rewardNote = tender.rewardNote?.trim() || "";
  const rewardTags = tender.rewardTags ?? [];
  const achievements = tender.rewardAchievements ?? [];
  const caps = tender.rewardCapabilities ?? [];
  const outcomeFailed =
    grade === "fail" ||
    /失败|failed|failure|error:|履约失败/i.test(outcome);
  const statusLabel = !outcome ? "执行中" : outcomeFailed ? "失败" : "已完成";
  const gradeLabel =
    grade === "excellent"
      ? "优秀"
      : grade === "pass"
        ? "合格"
        : grade === "thin"
          ? "偏薄"
          : grade === "fail"
            ? "不合格"
            : outcomeFailed
              ? "不合格"
              : grade;

  return (
    <div
      className={
        emphasis
          ? "rounded-2xl border border-primary/40 bg-primary/5 px-4 py-4 shadow-sm"
          : "rounded-2xl border border-border/60 bg-background px-4 py-3 shadow-sm"
      }
      data-testid={`tender-result-${tender.tenderId}`}
    >
      {title ? (
        <p className="text-muted-foreground mb-2 text-xs font-medium uppercase tracking-wide">
          {title}
        </p>
      ) : null}
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm text-muted-foreground">{tender.title}</p>
          {outcome ? (
            <p
              className={
                emphasis
                  ? "mt-2 whitespace-pre-wrap text-2xl font-semibold tracking-tight"
                  : "mt-2 whitespace-pre-wrap text-base font-semibold"
              }
              data-testid={`tender-outcome-${tender.tenderId}`}
            >
              {outcome}
            </p>
          ) : (
            <p className="text-muted-foreground mt-2 text-sm">
              已指派 @{winner}，等待任务结果…
            </p>
          )}
          <p className="text-muted-foreground mt-2 text-xs">
            执行者 @{winner} · {tender.capability}
            {tender.allocation
              ? ` · 中标报价 ¤${tender.allocation.winnerQuote}`
              : ""}
          </p>
          <TagRow tags={tender.tags} />
          <AllocationBoard allocation={tender.allocation} />
          {outcome ? (
            <div className="mt-3 flex flex-col gap-1.5">
              <div className="flex flex-wrap gap-1.5 text-xs">
                {gradeLabel ? (
                  <Badge variant="secondary">评级 {gradeLabel}</Badge>
                ) : null}
                {!outcomeFailed && gold != null ? (
                  <Badge variant="secondary">¤{gold} 金币</Badge>
                ) : null}
                {!outcomeFailed && rep != null ? (
                  <Badge variant="secondary">
                    声望 {rep > 0 ? `+${rep}` : rep}
                  </Badge>
                ) : null}
                {outcomeFailed && (gold != null || rep != null) ? (
                  <Badge variant="secondary">
                    结算 ¤{gold ?? 0} · 声望{" "}
                    {(rep ?? 0) > 0 ? `+${rep}` : (rep ?? 0)}
                  </Badge>
                ) : null}
                {tier ? <Badge variant="secondary">段位 {tier}</Badge> : null}
                {!outcomeFailed
                  ? rewardTags.map((tag) => (
                      <Badge key={`rw-${tag}`} variant="outline">
                        头衔 {tag}
                      </Badge>
                    ))
                  : null}
                {!outcomeFailed
                  ? achievements.map((id) => (
                      <Badge key={`ach-${id}`} variant="outline">
                        成就 {id}
                      </Badge>
                    ))
                  : null}
                {!outcomeFailed
                  ? caps.map((cap) => (
                      <Badge key={`cap-${cap}`} variant="outline">
                        新能力 {cap}
                      </Badge>
                    ))
                  : null}
              </div>
              {rewardNote ? (
                <p className="text-muted-foreground text-xs">{rewardNote}</p>
              ) : null}
            </div>
          ) : null}
        </div>
        <div className="flex shrink-0 flex-col items-end gap-2">
          <Badge variant={outcomeFailed ? "destructive" : "secondary"}>
            {statusLabel}
          </Badge>
          {onCancel ? (
            <Button
              onClick={() => onCancel(tender.tenderId)}
              size="sm"
              type="button"
              variant="ghost"
            >
              删除
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
