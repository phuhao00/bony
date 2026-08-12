/**
 * Official logos for the five Local Room seats (bundled under /room-agent-logos/).
 * Prefer these over the shared `grok` runtime HTTP icon most seats inherit.
 */
const ROOM_AGENT_AVATARS: Record<string, string> = {
  grok: "/room-agent-logos/grok.svg",
  zeroclaw: "/room-agent-logos/zeroclaw.png",
  unity: "/room-agent-logos/unity.svg",
  openmontage: "/room-agent-logos/openmontage.svg",
  docsmith: "/room-agent-logos/docsmith.png",
};

export function roomAgentAvatarUrl(
  name: string | null | undefined,
): string | null {
  if (!name?.trim()) return null;
  return ROOM_AGENT_AVATARS[name.trim().toLowerCase()] ?? null;
}

/** True when the stored URL is a shared/missing placeholder we should replace. */
export function shouldPreferRoomAgentAvatar(
  name: string | null | undefined,
  avatarUrl: string | null | undefined,
): boolean {
  const room = roomAgentAvatarUrl(name);
  if (!room) return false;
  const trimmed = avatarUrl?.trim() ?? "";
  if (!trimmed) return true;
  if (trimmed === room) return false;
  // Shared grok runtime HTTP icon used by every seat that runs `grok`.
  if (trimmed.includes("xai-org/grok-build") && trimmed.includes("logo.png")) {
    return true;
  }
  // Hand-rolled data-URI placeholders from earlier builds.
  if (trimmed.startsWith("data:image/svg+xml,")) {
    return true;
  }
  return !trimmed.startsWith("/room-agent-logos/");
}

export function resolveRoomAwareAvatarUrl(input: {
  name?: string | null;
  candidates: Array<string | null | undefined>;
}): string | null {
  const room = roomAgentAvatarUrl(input.name);
  for (const candidate of input.candidates) {
    const trimmed = candidate?.trim();
    if (!trimmed) continue;
    if (room && shouldPreferRoomAgentAvatar(input.name, trimmed)) {
      continue;
    }
    return trimmed;
  }
  return room;
}
