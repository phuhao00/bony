import type { ChannelAgentSessionAgent } from "./useChannelAgentSessions";

export function codingAgentRuntimeLabel(
  agent: ChannelAgentSessionAgent,
): string {
  const runtime = agent.runtime?.trim();
  if (runtime) return runtime;

  const command = agent.agentCommand?.trim();
  if (!command) return "ACP";
  const basename = command.split(/[\\/]/).pop() ?? command;
  return basename.replace(/\.(?:exe|cmd|bat)$/i, "") || "ACP";
}

export function isCodingWorkspaceCodingAgent(
  agent: ChannelAgentSessionAgent,
): boolean {
  return agent.capabilities.some((capability) =>
    capability.startsWith("code."),
  );
}

export function codingWorkspaceAgentRoleLabel(
  agent: ChannelAgentSessionAgent,
): string {
  if (isCodingWorkspaceCodingAgent(agent)) return "Coding agent";
  if (agent.capabilities.some((value) => value.startsWith("unity."))) {
    return "Tool agent";
  }
  if (agent.capabilities.some((value) => value.startsWith("research."))) {
    return "Research agent";
  }
  if (agent.capabilities.some((value) => value.startsWith("document."))) {
    return "Document agent";
  }
  if (agent.capabilities.some((value) => value.startsWith("media."))) {
    return "Media agent";
  }
  return "Agent";
}

export function selectCodingWorkspaceAgent(
  agents: ChannelAgentSessionAgent[],
  selectedPubkey: string | null,
): ChannelAgentSessionAgent | null {
  const available = agents.filter(
    (agent) => agent.agentSource === "managed" && agent.pubkey.trim(),
  );
  const fallback =
    available.find(isCodingWorkspaceCodingAgent) ?? available[0] ?? null;
  if (!selectedPubkey) return fallback;
  const normalized = selectedPubkey.toLowerCase();
  return (
    available.find((agent) => agent.pubkey.toLowerCase() === normalized) ??
    fallback
  );
}

export function withSelectedCodingAgentMention(
  mentionPubkeys: string[],
  selectedAgentPubkey: string,
  codingAgentPubkeys: string[],
): string[] {
  const codingAgents = new Set(
    codingAgentPubkeys.map((pubkey) => pubkey.toLowerCase()),
  );
  const selected = selectedAgentPubkey.toLowerCase();
  const result = mentionPubkeys.filter(
    (pubkey) => !codingAgents.has(pubkey.toLowerCase()),
  );
  if (!result.some((pubkey) => pubkey.toLowerCase() === selected)) {
    result.push(selectedAgentPubkey);
  }
  return result;
}
