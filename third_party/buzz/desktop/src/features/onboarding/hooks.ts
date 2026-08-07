import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  managedAgentsQueryKey,
  relayAgentsQueryKey,
} from "@/features/agents/hooks";
import { channelsQueryKey } from "@/features/channels/hooks";
import {
  ensureStarterChannels,
  ensureWelcomeChannel,
  hasEnsuredWelcomeChannel,
  markWelcomeChannelEnsured,
  notifyWelcomeChannelReady,
  rememberPendingWelcomeChannel,
} from "@/features/onboarding/welcome";
import { forceFreshOnboarding } from "@/features/onboarding/devFreshOnboarding";
import { ensureWelcomeCanvas } from "@/features/onboarding/welcomeCanvas";
import { ensureWelcomeTeam } from "@/features/onboarding/welcomeGuide";
import { useProfileQuery, useUpdateProfileMutation } from "@/features/profile/hooks";
import { emojiAvatarDataUrl } from "@/features/profile/ui/ProfileAvatarEditor.utils";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel } from "@/shared/api/types";
import {
  createChannel,
  deleteChannel,
  ensureStarterChannels as ensureStarterChannelsCommand,
  getChannelMembers,
  getChannels,
  seedRoomAgents,
  updateChannel,
} from "@/shared/api/tauri";

const STARTER_CHANNEL_SETUP_TOAST_ID = "starter-channel-setup-error";

export type ChannelInitResult =
  | { ok: true; focusChannelId?: string }
  | { ok: false; reason: string; focusChannelId?: string };

const welcomeSeedPromises = new Map<string, Promise<void>>();

function seedWelcomeExperience(
  queryClient: ReturnType<typeof useQueryClient>,
  channelId: string,
  pubkey: string | null,
  communityScope: string | null,
) {
  const key = `${communityScope ?? ""}:${channelId}`;
  const current = welcomeSeedPromises.get(key);
  if (current) return current;

  const promise = (async () => {
    try {
      await ensureWelcomeTeam(channelId, communityScope);
      await ensureWelcomeCanvas(channelId);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
        queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
      ]);
      markWelcomeChannelEnsured(pubkey, communityScope);
    } catch (error) {
      console.warn("Failed to seed the private Welcome experience.", error);
    }
  })().finally(() => welcomeSeedPromises.delete(key));
  welcomeSeedPromises.set(key, promise);
  return promise;
}

export async function initializeStarterChannels(
  queryClient: ReturnType<typeof useQueryClient>,
  {
    focus,
    pubkey,
    communityScope,
  }: {
    focus: boolean;
    pubkey: string | null;
    communityScope: string | null;
  },
): Promise<ChannelInitResult> {
  try {
    let starterChannels: Awaited<
      ReturnType<typeof ensureStarterChannels>
    > | null = null;
    let starterChannelsError: unknown = null;
    try {
      starterChannels = await ensureStarterChannels({
        ensureStarterChannels: ensureStarterChannelsCommand,
        getChannels,
      });
    } catch (error) {
      starterChannelsError = error;
      console.warn("Failed to initialize public starter channels.", error);
    }

    const welcomeChannel = await ensureWelcomeChannel(
      {
        createChannel,
        deleteChannel,
        getChannelMembers,
        getChannels,
        updateChannel,
      },
      {
        replaceExisting: forceFreshOnboarding,
      },
    );

    const starterChannelList = starterChannels?.channels ?? [];
    queryClient.setQueryData<Channel[]>(channelsQueryKey, (channels = []) => {
      const ensuredIds = new Set(
        starterChannelList.map((channel) => channel.id),
      );
      ensuredIds.add(welcomeChannel.id);
      return [
        ...starterChannelList,
        ...(starterChannelList.some(
          (channel) => channel.id === welcomeChannel.id,
        )
          ? []
          : [welcomeChannel]),
        ...channels.filter((channel) => !ensuredIds.has(channel.id)),
      ];
    });
    void seedWelcomeExperience(
      queryClient,
      welcomeChannel.id,
      pubkey,
      communityScope,
    );
    await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
    if (focus) {
      // Refreshing can briefly replace the optimistic cache with an older relay
      // snapshot. Reinsert the just-ensured channels before announcing focus so
      // the route can consume the pending private Welcome channel immediately.
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (channels = []) => {
        const byId = new Map(
          [...channels, ...starterChannelList, welcomeChannel].map(
            (channel) => [channel.id, channel],
          ),
        );
        return [...byId.values()];
      });
      rememberPendingWelcomeChannel(welcomeChannel.id);
      notifyWelcomeChannelReady(welcomeChannel.id);
    }
    const focusChannelId = focus ? welcomeChannel.id : undefined;
    if (starterChannelsError) {
      return {
        ok: false,
        focusChannelId,
        reason:
          starterChannelsError instanceof Error
            ? starterChannelsError.message
            : "Failed to set up starter channels",
      };
    }
    return { ok: true, focusChannelId };
  } catch (error) {
    console.warn("Failed to initialize starter channels.", error);
    return {
      ok: false,
      reason:
        error instanceof Error
          ? error.message
          : "Failed to set up starter channels",
    };
  }
}

type AppOnboardingStage = "blocking" | "ready" | "reset-failed";

const DEFAULT_PROFILE_DISPLAY_NAME = "Local";
const DEFAULT_PROFILE_AVATAR_EMOJI = "🐝";
const DEFAULT_PROFILE_AVATAR_COLOR = "#FFB84D";

// Per-pubkey guard so the silent default-profile write below fires at most
// once per identity per app session, even across effect re-runs / remounts.
const defaultProfileWriteAttempted = new Set<string>();

// Per-pubkey guard mirroring `defaultProfileWriteAttempted` for the native
// room-agent seed below — at most one `seed_room_agents` call per identity
// per app session.
const roomAgentsSeedAttempted = new Set<string>();

/**
 * Single-machine local build: there is no "set up your profile" wizard
 * anymore. If the relay has no kind:0 event for the current identity yet
 * (first-ever launch, or a fresh identity), silently write a default display
 * name + emoji avatar and move straight into the app. Settings → Profile
 * remains the entry point for anyone who wants to change it.
 */
export function useAppOnboardingState(isSharedIdentity: boolean) {
  const queryClient = useQueryClient();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const identity = identityQuery.data;
  const currentPubkey = identity?.pubkey ?? null;
  const identityResetFailed = identity?.resetFailed === true;
  const starterChannelsCommunityScope = activeCommunity?.relayUrl ?? null;
  const starterChannelsInitPromisesRef = React.useRef(
    new Map<string, Promise<ChannelInitResult>>(),
  );
  const starterChannelsFocusIntentRef = React.useRef(
    new Map<string, boolean>(),
  );

  const profileQuery = useProfileQuery(identityQuery.status === "success");
  const updateProfileMutation = useUpdateProfileMutation();
  const [isWritingDefaultProfile, setIsWritingDefaultProfile] =
    React.useState(false);

  React.useEffect(() => {
    if (
      !currentPubkey ||
      profileQuery.status !== "success" ||
      profileQuery.data?.hasProfileEvent !== false ||
      defaultProfileWriteAttempted.has(currentPubkey)
    ) {
      return;
    }
    defaultProfileWriteAttempted.add(currentPubkey);
    setIsWritingDefaultProfile(true);
    void updateProfileMutation
      .mutateAsync({
        avatarUrl: emojiAvatarDataUrl(
          DEFAULT_PROFILE_AVATAR_EMOJI,
          DEFAULT_PROFILE_AVATAR_COLOR,
        ),
        displayName: DEFAULT_PROFILE_DISPLAY_NAME,
      })
      .catch((error) => {
        console.warn("Failed to write the default local profile.", error);
      })
      .finally(() => setIsWritingDefaultProfile(false));
  }, [
    currentPubkey,
    profileQuery.data?.hasProfileEvent,
    profileQuery.status,
    updateProfileMutation,
  ]);

  // Single-machine local build: the five room agents (Grok, ZeroClaw, Unity,
  // OpenMontage, DocSmith) and their shared "Local Room" channel used to be
  // provisioned by an external PowerShell script chain before Desktop even
  // launched. `seed_room_agents` is native, idempotent Rust — call it once
  // per identity, right after identity is ready, and let Desktop's own
  // managed-agent lifecycle (start/stop with the app) take it from there.
  React.useEffect(() => {
    if (
      identityQuery.status !== "success" ||
      identityResetFailed ||
      !currentPubkey ||
      roomAgentsSeedAttempted.has(currentPubkey)
    ) {
      return;
    }
    roomAgentsSeedAttempted.add(currentPubkey);
    void seedRoomAgents()
      .then((result) => {
        if (result.errors.length > 0) {
          console.warn("seed_room_agents reported errors:", result.errors);
        }
        if (result.createdAgents.length > 0 || result.createdChannel) {
          void Promise.all([
            queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
            queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
            queryClient.invalidateQueries({ queryKey: channelsQueryKey }),
          ]);
        }
      })
      .catch((error) => {
        console.warn("Failed to seed local room agents.", error);
      });
  }, [currentPubkey, identityQuery.status, identityResetFailed, queryClient]);

  const requestStarterChannels = React.useCallback(
    (focus: boolean): Promise<ChannelInitResult> => {
      if (!currentPubkey || !starterChannelsCommunityScope) {
        return Promise.resolve({ ok: true });
      }

      const starterChannelsInitKey = `${starterChannelsCommunityScope}:${currentPubkey}`;
      const currentPromise = starterChannelsInitPromisesRef.current.get(
        starterChannelsInitKey,
      );
      if (currentPromise) {
        // A focus=true request must not be swallowed behind an in-flight
        // focus=false promise. Upgrade the intent: when the background
        // promise resolves, chain a focus-only follow-up.
        if (
          focus &&
          !starterChannelsFocusIntentRef.current.get(starterChannelsInitKey)
        ) {
          starterChannelsFocusIntentRef.current.set(
            starterChannelsInitKey,
            true,
          );
          return currentPromise.then((result) => {
            if (!result.ok) return result;
            return initializeStarterChannels(queryClient, {
              focus: true,
              pubkey: currentPubkey,
              communityScope: starterChannelsCommunityScope,
            });
          });
        }
        return currentPromise;
      }

      if (focus) {
        starterChannelsFocusIntentRef.current.set(starterChannelsInitKey, true);
      }
      const promise = initializeStarterChannels(queryClient, {
        focus,
        pubkey: currentPubkey,
        communityScope: starterChannelsCommunityScope,
      }).finally(() => {
        starterChannelsInitPromisesRef.current.delete(starterChannelsInitKey);
        starterChannelsFocusIntentRef.current.delete(starterChannelsInitKey);
      });
      starterChannelsInitPromisesRef.current.set(
        starterChannelsInitKey,
        promise,
      );
      return promise;
    },
    [currentPubkey, queryClient, starterChannelsCommunityScope],
  );

  const showStarterRetryToast = React.useCallback(
    (reason: string) => {
      toast.error("Couldn't set up starter channels", {
        id: STARTER_CHANNEL_SETUP_TOAST_ID,
        action: {
          label: "Retry",
          onClick: (event) => {
            event.preventDefault();
            void requestStarterChannels(true).then((result) => {
              if (!result.ok) {
                window.setTimeout(
                  // Sonner dismisses an action toast as its click resolves, so
                  // recreate a failed retry after that dismissal completes.
                  () => showStarterRetryToast(result.reason),
                  0,
                );
                return;
              }
              toast.dismiss(STARTER_CHANNEL_SETUP_TOAST_ID);
            });
          },
        },
        description: reason,
      });
    },
    [requestStarterChannels],
  );

  const identityReady = identityQuery.status === "success" && !identityResetFailed;
  const profileSettled =
    profileQuery.status === "success" || profileQuery.status === "error";

  React.useEffect(() => {
    if (
      !identityReady ||
      !profileSettled ||
      isWritingDefaultProfile ||
      !currentPubkey ||
      !starterChannelsCommunityScope ||
      hasEnsuredWelcomeChannel(currentPubkey, starterChannelsCommunityScope)
    ) {
      return;
    }

    void requestStarterChannels(false).then((result) => {
      if (!result.ok) {
        showStarterRetryToast(result.reason);
      }
    });
  }, [
    currentPubkey,
    identityReady,
    isSharedIdentity,
    isWritingDefaultProfile,
    profileSettled,
    requestStarterChannels,
    showStarterRetryToast,
    starterChannelsCommunityScope,
  ]);

  const stage: AppOnboardingStage = identityResetFailed
    ? "reset-failed"
    : identityReady && profileSettled && !isWritingDefaultProfile
      ? "ready"
      : "blocking";

  return {
    currentPubkey,
    stage,
  };
}
