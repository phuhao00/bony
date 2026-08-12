import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import {
  parseProfilePanelTab,
  parseProfilePanelView,
  type ProfilePanelTab,
  type ProfilePanelView,
} from "@/features/profile/ui/UserProfilePanelUtils";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

type EconomyRouteSearch = {
  profile?: string;
  profileTab?: ProfilePanelTab;
  profileView?: ProfilePanelView;
};

function nonEmptyString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function validateEconomySearch(
  search: Record<string, unknown>,
): EconomyRouteSearch {
  return {
    profile: nonEmptyString(search.profile),
    profileTab: parseProfilePanelTab(search.profileTab) ?? undefined,
    profileView: parseProfilePanelView(search.profileView) ?? undefined,
  };
}

const EconomyScreen = React.lazy(async () => {
  const module = await import("@/features/agents/ui/EconomyScreen");
  return { default: module.EconomyScreen };
});

export const Route = createFileRoute("/economy")({
  validateSearch: validateEconomySearch,
  component: EconomyRouteComponent,
});

function EconomyRouteComponent() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
      <EconomyScreen />
    </React.Suspense>
  );
}
