import {
  useEconomyAdminMutation,
  useEconomyTendersQuery,
} from "@/features/agents/hooks";
import { suggestEconomyTender, sweepEconomyTenders } from "@/shared/api/tauri";
import type { TenderSnapshot, TenderSuggestion } from "@/shared/api/tauri";
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
  const busy =
    admin.publishTender.isPending ||
    admin.inviteTenderBids.isPending ||
    admin.resolveTender.isPending ||
    admin.sweepTenders.isPending;

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

  return (
    <section
      aria-label="Tender market"
      className="flex flex-col gap-4"
      data-testid="economy-tender-market"
    >
      <div className="rounded-2xl border border-border/70 bg-muted/15 p-4">
        <p className="mb-1 text-sm font-medium">发一个任务</p>
        <p className="text-muted-foreground mb-3 text-xs">
          只需填写标题。系统会自动选能力、定预算、邀请匹配 Agent，选出赢家并执行任务，把答案显示在下方。
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
          <p
            className="text-muted-foreground mt-3 text-xs tabular-nums"
            data-testid="tender-suggestion-preview"
          >
            将使用 {suggestion.capability} · 预算 ¤{suggestion.budget}
          </p>
        ) : null}
      </div>

      {actionError ? (
        <p className="text-destructive text-sm">{actionError}</p>
      ) : null}

      {latestResult?.status === "resolved" ? (
        <ResultCard
          emphasis
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
        <p className="text-sm font-medium">历史结果</p>
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
                  <ResultCard tender={tender} />
                </li>
              ))}
          </ul>
        )}
      </div>

      {stuckOpen.length > 0 ? (
        <div className="space-y-2">
          <div className="flex items-center justify-between gap-2">
            <p className="text-sm font-medium">未匹配到 Agent 的任务</p>
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
          </div>
          <ul className="flex flex-col gap-2">
            {stuckOpen.map((tender) => (
              <li
                className="rounded-2xl border border-border/60 bg-background px-4 py-3 text-sm shadow-sm"
                key={tender.tenderId}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium">{tender.title}</span>
                  <Badge variant="secondary">等待匹配</Badge>
                </div>
                <p className="text-muted-foreground mt-1 text-xs">
                  需要能力 {tender.capability} · 预算 ¤{tender.budget}
                  （当前没有声明该能力的 Agent）
                </p>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </section>
  );
}

function ResultCard({
  tender,
  title,
  emphasis = false,
}: {
  tender: TenderSnapshot;
  title?: string;
  emphasis?: boolean;
}) {
  const winner = tender.winnerName?.trim() || "未知";
  const outcome = tender.outcome?.trim() || "";
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
          <p className="text-muted-foreground mt-2 text-xs tabular-nums">
            执行者 @{winner} · 预算 ¤{tender.budget} · {tender.capability}
          </p>
        </div>
        <Badge variant="secondary">{outcome ? "已完成" : "执行中"}</Badge>
      </div>
    </div>
  );
}
