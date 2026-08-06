import type { Community } from "./types";
import { homeDir } from "@tauri-apps/api/path";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

const COMMUNITIES_KEY = "buzz-communities";
const ACTIVE_COMMUNITY_KEY = "buzz-active-community-id";
const LEGACY_WORKSPACES_KEY = "buzz-workspaces";
const LEGACY_ACTIVE_WORKSPACE_KEY = "buzz-active-workspace-id";

/**
 * Expand a leading `~` to the user's home directory. The backend rejects
 * `~`-prefixed paths (`std::fs` does not expand the shell tilde), so the UI
 * resolves it before save. Returns non-`~` input unchanged. Empty/whitespace
 * input returns `undefined` so callers can clear the override.
 */
export async function expandTilde(input: string): Promise<string | undefined> {
  const trimmed = input.trim();
  if (!trimmed) {
    return undefined;
  }
  if (trimmed === "~") {
    return homeDir();
  }
  if (trimmed.startsWith("~/")) {
    const home = await homeDir();
    const base = home.endsWith("/") ? home.slice(0, -1) : home;
    return `${base}/${trimmed.slice(2)}`;
  }
  return trimmed;
}

export function migrateLegacyCommunityStorage(
  storage: Storage = localStorage,
): void {
  if (storage.getItem(COMMUNITIES_KEY) === null) {
    const legacyCommunities = storage.getItem(LEGACY_WORKSPACES_KEY);
    if (legacyCommunities !== null) {
      storage.setItem(COMMUNITIES_KEY, legacyCommunities);
    }
  }
  if (storage.getItem(ACTIVE_COMMUNITY_KEY) === null) {
    const legacyActiveCommunity = storage.getItem(LEGACY_ACTIVE_WORKSPACE_KEY);
    if (legacyActiveCommunity !== null) {
      storage.setItem(ACTIVE_COMMUNITY_KEY, legacyActiveCommunity);
    }
  }
}

export function loadCommunities(): Community[] {
  try {
    migrateLegacyCommunityStorage();
    const raw = localStorage.getItem(COMMUNITIES_KEY);
    if (!raw) {
      return [];
    }
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    // Migration: older builds stored the user's `nsec` in localStorage and
    // re-applied it to the backend on every reload, which silently overwrote
    // any `import_identity` result with the original generated key. The
    // on-disk `identity.key` file is the only source of truth now. Strip
    // any lingering `nsec` from existing entries on read and persist the
    // cleaned list back so it cannot leak into future sessions.
    let didStrip = false;
    const cleaned = (parsed as Array<Record<string, unknown>>).map((entry) => {
      if (entry && typeof entry === "object" && "nsec" in entry) {
        const { nsec: _nsec, ...rest } = entry;
        didStrip = true;
        return rest;
      }
      return entry;
    }) as Community[];
    if (didStrip) {
      setLocalStorageItemWithRecovery(COMMUNITIES_KEY, JSON.stringify(cleaned));
    }
    return cleaned;
  } catch {
    return [];
  }
}

export function saveCommunities(communities: Community[]): boolean {
  return setLocalStorageItemWithRecovery(
    COMMUNITIES_KEY,
    JSON.stringify(communities),
  );
}

export function clearCommunityStorage(storage: Storage = localStorage): void {
  storage.removeItem(COMMUNITIES_KEY);
  storage.removeItem(ACTIVE_COMMUNITY_KEY);
  storage.removeItem(LEGACY_WORKSPACES_KEY);
  storage.removeItem(LEGACY_ACTIVE_WORKSPACE_KEY);
}

export function loadActiveCommunityId(): string | null {
  migrateLegacyCommunityStorage();
  return localStorage.getItem(ACTIVE_COMMUNITY_KEY);
}

export function saveActiveCommunityId(id: string): boolean {
  return setLocalStorageItemWithRecovery(ACTIVE_COMMUNITY_KEY, id);
}

export function normalizeRelayUrl(url: string): string {
  if (!url.startsWith("ws://") && !url.startsWith("wss://")) {
    return `wss://${url}`;
  }
  return url;
}

function isLocalRelayHost(hostname: string): boolean {
  return ["localhost", "127.0.0.1", "[::1]", "0.0.0.0"].includes(hostname);
}

export function shouldAutoConnectDefaultRelay(relayUrl: string): boolean {
  try {
    const parsed = new URL(relayUrl);
    // Local monorepo stacks (ws://localhost:3000) must auto-connect: excluding
    // loopback left create-channel orphaned against a missing first community
    // or a stale production community still in localStorage.
    return parsed.protocol === "ws:" || parsed.protocol === "wss:";
  } catch {
    return false;
  }
}

/** True when the relay host is a loopback address used by the local room stack. */
export function isLocalRelayUrl(relayUrl: string): boolean {
  try {
    const normalized = normalizeRelayUrl(relayUrl);
    const parsed = new URL(
      normalized.replace("ws://", "http://").replace("wss://", "https://"),
    );
    return isLocalRelayHost(parsed.hostname);
  } catch {
    return false;
  }
}

function relayUrlKey(relayUrl: string): string {
  return normalizeRelayUrl(relayUrl).replace(/\/$/, "").toLowerCase();
}

/**
 * When Desktop is launched against a loopback default relay, ensure a Local Dev
 * community exists and is active so normal create-channel / add-agent lands on
 * the same open relay the room agents listen to.
 */
export function ensureLocalCommunityActive(args: {
  defaultRelayUrl: string;
  identityPubkey: string;
  communities: Community[];
  activeId: string | null;
}): { communities: Community[]; activeId: string | null; changed: boolean } {
  if (!isLocalRelayUrl(args.defaultRelayUrl)) {
    return {
      communities: args.communities,
      activeId: args.activeId,
      changed: false,
    };
  }

  const preferred = normalizeRelayUrl(args.defaultRelayUrl);
  // Always use `localhost` for loopback monorepo communities. 127.0.0.1 is a
  // different Host for buzz-relay multi-tenant and yields WebSocket 404, so
  // managed agents never hear @mentions even when the UI works.
  const preferredCanonical = preferred
    .replace("://127.0.0.1", "://localhost")
    .replace("://[::1]", "://localhost")
    .replace("://0.0.0.0", "://localhost");
  const preferredKey = relayUrlKey(preferredCanonical);
  let communities = args.communities.slice();
  let changed = false;

  // Rewrite any 127.0.0.1 / ::1 entries onto the canonical preferred host.
  communities = communities.map((c) => {
    if (!isLocalRelayUrl(c.relayUrl)) {
      return c;
    }
    const next = normalizeRelayUrl(c.relayUrl)
      .replace("://127.0.0.1", "://localhost")
      .replace("://[::1]", "://localhost")
      .replace("://0.0.0.0", "://localhost");
    if (next === c.relayUrl) {
      return c;
    }
    changed = true;
    return { ...c, relayUrl: next };
  });

  let local =
    communities.find((c) => relayUrlKey(c.relayUrl) === preferredKey) ??
    communities.find((c) => isLocalRelayUrl(c.relayUrl));

  if (!local) {
    local = {
      id: crypto.randomUUID(),
      name: deriveCommunityName(preferredCanonical),
      relayUrl: preferredCanonical,
      pubkey: args.identityPubkey,
      addedAt: new Date().toISOString(),
    };
    communities = [...communities, local];
    changed = true;
  } else if (relayUrlKey(local.relayUrl) !== preferredKey) {
    // Prefer the env default host when several locals exist.
    local = { ...local, relayUrl: preferredCanonical };
    communities = communities.map((c) =>
      c.id === local!.id ? local! : c,
    );
    changed = true;
  }

  const active = communities.find((c) => c.id === args.activeId);
  if (!active || !isLocalRelayUrl(active.relayUrl)) {
    return { communities, activeId: local.id, changed: true };
  }

  return { communities, activeId: args.activeId, changed };
}

export function deriveCommunityName(relayUrl: string): string {
  try {
    const url = new URL(
      relayUrl.replace("ws://", "http://").replace("wss://", "https://"),
    );
    const host = url.hostname;
    if (isLocalRelayHost(host)) {
      return "Local Dev";
    }
    const parts = host.split(".");
    // Detect staging environments (e.g. buzz-oss.stage.blox.sqprod.co)
    if (parts.some((p) => p === "stage" || p === "staging")) {
      return "Buzz (staging)";
    }
    // Use the first subdomain segment or the domain itself
    if (parts.length >= 2) {
      return parts[0] === "relay" ? parts[1] : parts[0];
    }
    return host;
  } catch {
    return "Community";
  }
}

export function initFirstCommunity(
  relayUrl: string,
  pubkey: string,
  name?: string,
): Community | null {
  const normalizedUrl = normalizeRelayUrl(relayUrl);
  const trimmedName = name?.trim();
  const community: Community = {
    id: crypto.randomUUID(),
    name: trimmedName || deriveCommunityName(normalizedUrl),
    relayUrl: normalizedUrl,
    // Compiled default relays must admit the first token-less connection; there
    // is no invite-token prompt on this auto-connect path.
    pubkey,
    addedAt: new Date().toISOString(),
  };
  const previousActiveCommunityId = localStorage.getItem(ACTIVE_COMMUNITY_KEY);
  const didSaveActiveCommunity = saveActiveCommunityId(community.id);
  if (!didSaveActiveCommunity) {
    return null;
  }

  if (!saveCommunities([community])) {
    // A failed setItem leaves the existing communities value untouched. Roll
    // back only the active-ID write so inconsistent pre-existing data is never
    // destroyed while recovering from a quota failure.
    try {
      if (previousActiveCommunityId === null) {
        localStorage.removeItem(ACTIVE_COMMUNITY_KEY);
      } else {
        localStorage.setItem(ACTIVE_COMMUNITY_KEY, previousActiveCommunityId);
      }
    } catch {
      // Best effort: persistence is already unavailable, and callers will stay
      // on setup instead of reloading.
    }
    return null;
  }

  return community;
}
