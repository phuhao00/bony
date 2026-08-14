import assert from "node:assert/strict";
import test from "node:test";

import {
  codingAgentRuntimeLabel,
  codingWorkspaceAgentRoleLabel,
  isCodingWorkspaceCodingAgent,
  selectCodingWorkspaceAgent,
  withSelectedCodingAgentMention,
} from "./codingWorkspaceSurface.ts";

function agent(overrides = {}) {
  return {
    pubkey: "grok-key",
    name: "Grok",
    runtime: null,
    status: "running",
    agentCommand: null,
    agentSource: "managed",
    canInterruptTurn: true,
    model: "grok-code-fast-1",
    provider: "xai",
    capabilities: ["code.repo.read", "code.rust.change"],
    ...overrides,
  };
}

test("selects a managed coding agent by stable pubkey", () => {
  const agents = [
    agent({ pubkey: "relay", agentSource: "relay" }),
    agent({
      pubkey: "docsmith",
      name: "DocSmith",
      capabilities: ["document.create"],
    }),
    agent({ pubkey: "grok" }),
    agent({ pubkey: "codex", name: "Codex", capabilities: [] }),
  ];
  assert.equal(selectCodingWorkspaceAgent(agents, "CODEX")?.pubkey, "codex");
  assert.equal(selectCodingWorkspaceAgent(agents, "missing")?.pubkey, "grok");
});

test("role labels follow stable capabilities instead of display names", () => {
  const toolSeat = agent({ name: "Anything", capabilities: ["document.create"] });
  const renamedCoder = agent({ name: "Not Grok" });
  assert.equal(codingWorkspaceAgentRoleLabel(toolSeat), "Tool agent");
  assert.equal(isCodingWorkspaceCodingAgent(toolSeat), false);
  assert.equal(codingWorkspaceAgentRoleLabel(renamedCoder), "Coding agent");
  assert.equal(isCodingWorkspaceCodingAgent(renamedCoder), true);
});

test("runtime label follows runtime metadata then executable basename", () => {
  assert.equal(codingAgentRuntimeLabel(agent({ runtime: "claude" })), "claude");
  assert.equal(
    codingAgentRuntimeLabel(agent({ agentCommand: "C:\\tools\\codex.cmd" })),
    "codex",
  );
  assert.equal(codingAgentRuntimeLabel(agent()), "ACP");
});

test("task dispatch keeps exactly one coding agent mention", () => {
  assert.deepEqual(
    withSelectedCodingAgentMention(["human", "grok", "CLAUDE"], "codex", [
      "grok",
      "codex",
      "claude",
    ]),
    ["human", "codex"],
  );
});
