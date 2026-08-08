import assert from "node:assert/strict";
import test from "node:test";

import {
  codingAgentRuntimeLabel,
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
    ...overrides,
  };
}

test("selects a managed coding agent by stable pubkey", () => {
  const agents = [
    agent({ pubkey: "relay", agentSource: "relay" }),
    agent({ pubkey: "grok" }),
    agent({ pubkey: "codex", name: "Codex" }),
  ];
  assert.equal(selectCodingWorkspaceAgent(agents, "CODEX")?.pubkey, "codex");
  assert.equal(selectCodingWorkspaceAgent(agents, "missing")?.pubkey, "grok");
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
