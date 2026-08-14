import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const MarketScreen = React.lazy(async () => {
  const module = await import("@/features/agents/ui/MarketScreen");
  return { default: module.MarketScreen };
});

export const Route = createFileRoute("/market")({
  component: MarketRouteComponent,
});

function MarketRouteComponent() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
      <MarketScreen />
    </React.Suspense>
  );
}
