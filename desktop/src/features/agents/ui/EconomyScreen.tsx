import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { EconomyLeaderboardPanel } from "@/features/agents/ui/EconomyLeaderboardPanel";
import { useOpenDmMutation } from "@/features/channels/hooks";
import {
  type ProfilePanelTab,
  type ProfilePanelView,
  UserProfilePanel,
} from "@/features/profile/ui/UserProfilePanel";
import {
  profilePanelTabFromSearch,
  profilePanelViewFromSearch,
} from "@/features/profile/ui/UserProfilePanelUtils";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  type ProfilePanelOpenOptions,
  ProfilePanelProvider,
} from "@/shared/context/ProfilePanelContext";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import { PageHeader } from "@/shared/ui/PageHeader";

const ECONOMY_PROFILE_SEARCH_KEYS = [
  "profile",
  "profileTab",
  "profileView",
] as const;

export function EconomyScreen() {
  const identityQuery = useIdentityQuery();
  const { applyPatch, values } = useHistorySearchState(
    ECONOMY_PROFILE_SEARCH_KEYS,
  );
  const profilePanelTab = profilePanelTabFromSearch(values.profileTab);
  const profilePanelView = profilePanelViewFromSearch(values.profileView);
  const profilePanelPubkey = values.profile;
  const threadPanelWidth = useThreadPanelWidth();
  const openDmMutation = useOpenDmMutation();
  const { goChannel } = useAppNavigation();

  const handleOpenProfilePanel = React.useCallback(
    (pubkey: string, options?: ProfilePanelOpenOptions) => {
      applyPatch({
        profile: pubkey,
        profileTab: options?.tab === "info" ? null : (options?.tab ?? null),
        profileView: null,
      });
    },
    [applyPatch],
  );
  const handleCloseProfilePanel = React.useCallback(() => {
    applyPatch({
      profile: null,
      profileTab: null,
      profileView: null,
    });
  }, [applyPatch]);
  const handleProfilePanelViewChange = React.useCallback(
    (view: ProfilePanelView, options?: { replace?: boolean }) =>
      applyPatch({ profileView: view === "summary" ? null : view }, options),
    [applyPatch],
  );
  const handleProfilePanelTabChange = React.useCallback(
    (tab: ProfilePanelTab, options?: { replace?: boolean }) =>
      applyPatch({ profileTab: tab === "info" ? null : tab }, options),
    [applyPatch],
  );
  const handleOpenDm = React.useCallback(
    async (pubkeys: string[]) => {
      const dm = await openDmMutation.mutateAsync({ pubkeys });
      await goChannel(dm.id);
    },
    [goChannel, openDmMutation],
  );

  return (
    <ProfilePanelProvider onOpenProfilePanel={handleOpenProfilePanel}>
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <div className="flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden">
          <div className="flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8">
            <div className="mx-auto flex w-full max-w-5xl flex-col gap-6">
              <PageHeader
                description="Virtual credits, reputation tiers, and multi-board standings for agents and orgs."
                title="Room Economy"
              />
              <EconomyLeaderboardPanel />
            </div>
          </div>
          {profilePanelPubkey ? (
            <UserProfilePanel
              canResetWidth={threadPanelWidth.canReset}
              currentPubkey={identityQuery.data?.pubkey}
              onClose={handleCloseProfilePanel}
              onOpenDm={handleOpenDm}
              onOpenProfile={handleOpenProfilePanel}
              onResetWidth={threadPanelWidth.onResetWidth}
              onResizeStart={threadPanelWidth.onResizeStart}
              onTabChange={handleProfilePanelTabChange}
              onViewChange={handleProfilePanelViewChange}
              pubkey={profilePanelPubkey}
              tab={profilePanelTab}
              view={profilePanelView}
              widthPx={threadPanelWidth.widthPx}
            />
          ) : null}
        </div>
      </div>
    </ProfilePanelProvider>
  );
}
