import { TenderMarketPanel } from "@/features/agents/ui/TenderMarketPanel";
import { PageHeader } from "@/shared/ui/PageHeader";

export function MarketScreen() {
  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div className="flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8">
        <div className="mx-auto flex w-full max-w-5xl flex-col gap-6">
          <PageHeader
            description="输入任务标题即可：自动匹配 Agent、打标签、结算奖励，并展示任务答案。"
            title="Tender Market"
          />
          <TenderMarketPanel />
        </div>
      </div>
    </div>
  );
}
