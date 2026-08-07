import { useIdentityQuery } from "@/shared/api/hooks";

type MachineOnboardingStage = "blocking" | "ready" | "reset-failed";

function identitySettled(status: string, isFetching: boolean) {
  return !isFetching && (status === "success" || status === "error");
}

/**
 * Single-machine local build: identity is resolved (and, if missing,
 * silently generated + persisted) entirely on the Rust side — see
 * `resolve_persisted_identity` in app_state.rs. There is no user-facing
 * "choose new vs. import identity" wizard anymore, so this hook only needs
 * to report whether the identity is still loading, ready, or (rarely) stuck
 * in a failed sign-out/reset that requires a relaunch.
 */
export function useMachineOnboardingState() {
  const identityQuery = useIdentityQuery();
  const identity = identityQuery.data;
  const currentPubkey = identity?.pubkey ?? null;
  const identityResetFailed = identity?.resetFailed === true;

  let stage: MachineOnboardingStage;
  if (identityResetFailed && identityQuery.status === "success") {
    stage = "reset-failed";
  } else if (
    identitySettled(identityQuery.status, identityQuery.fetchStatus === "fetching")
  ) {
    stage = "ready";
  } else {
    stage = "blocking";
  }

  return {
    currentPubkey,
    stage,
  };
}
