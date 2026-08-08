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

export function selectCodingWorkspaceAgent(
  agents: ChannelAgentSessionAgent[],
  selectedPubkey: string | null,
): ChannelAgentSessionAgent | null {
  const available = agents.filter(
    (agent) => agent.agentSource === "managed" && agent.pubkey.trim(),
  );
  if (!selectedPubkey) return available[0] ?? null;
  const normalized = selectedPubkey.toLowerCase();
  return (
    available.find((agent) => agent.pubkey.toLowerCase() === normalized) ??
    available[0] ??
    null
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
