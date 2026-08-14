import assert from "node:assert/strict";
import test from "node:test";

import {
  clearCommunityStorage,
  ensureLocalCommunityActive,
  initFirstCommunity,
  migrateLegacyCommunityStorage,
  shouldAutoConnectDefaultRelay,
} from "./communityStorage.ts";

function createMemoryStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    values,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    clear: () => values.clear(),
    key: (index) => Array.from(values.keys())[index] ?? null,
    get length() {
      return values.size;
    },
  };
}

test("migrateLegacyCommunityStorage promotes current Buzz workspace state", () => {
  const storage = createMemoryStorage({
    "buzz-workspaces": '[{"id":"current"}]',
    "buzz-active-workspace-id": "current",
  });

  migrateLegacyCommunityStorage(storage);

  assert.equal(storage.getItem("buzz-communities"), '[{"id":"current"}]');
  assert.equal(storage.getItem("buzz-active-community-id"), "current");
});

test("migrateLegacyCommunityStorage does not overwrite new community state", () => {
  const storage = createMemoryStorage({
    "buzz-communities": '[{"id":"new"}]',
    "buzz-active-community-id": "new",
    "buzz-workspaces": '[{"id":"old"}]',
    "buzz-active-workspace-id": "old",
  });

  migrateLegacyCommunityStorage(storage);

  assert.equal(storage.getItem("buzz-communities"), '[{"id":"new"}]');
  assert.equal(storage.getItem("buzz-active-community-id"), "new");
});

test("signed-build relay defaults auto-connect during first-run onboarding", () => {
  assert.equal(
    shouldAutoConnectDefaultRelay("wss://buzz.block.builderlab.xyz"),
    true,
  );
  // Local monorepo stack must also auto-connect so create-channel lands on
  // the open ws://localhost:3000 relay (previously excluded, which orphaned UI).
  assert.equal(shouldAutoConnectDefaultRelay("ws://localhost:3000"), true);
  assert.equal(shouldAutoConnectDefaultRelay("ws://127.0.0.1:3000"), true);
  assert.equal(shouldAutoConnectDefaultRelay("ws://[::1]:3000"), true);
  assert.equal(shouldAutoConnectDefaultRelay("ws://0.0.0.0:3000"), true);
  assert.equal(shouldAutoConnectDefaultRelay("http://localhost:3000"), false);
  assert.equal(
    shouldAutoConnectDefaultRelay("https://relay.example.com"),
    false,
  );
  assert.equal(shouldAutoConnectDefaultRelay("relay.example.com"), false);
  assert.equal(shouldAutoConnectDefaultRelay("not a valid relay"), false);
});

test("failed first-community write preserves existing community data", () => {
  const storage = createMemoryStorage({
    "buzz-communities": '[{"id":"existing"}]',
    "buzz-workspaces": '[{"id":"legacy"}]',
    "buzz-active-workspace-id": "legacy",
  });
  storage.setItem = (key, value) => {
    if (key === "buzz-communities") {
      throw new Error("QuotaExceededError");
    }
    storage.values.set(key, String(value));
  };
  globalThis.localStorage = storage;
  globalThis.window = { localStorage: storage };

  assert.equal(initFirstCommunity("wss://relay.example.com", "pubkey"), null);
  assert.equal(storage.getItem("buzz-communities"), '[{"id":"existing"}]');
  assert.equal(storage.getItem("buzz-active-community-id"), null);
  assert.equal(storage.getItem("buzz-workspaces"), '[{"id":"legacy"}]');
  assert.equal(storage.getItem("buzz-active-workspace-id"), "legacy");
});

test("clearCommunityStorage removes new and legacy state", () => {
  const storage = createMemoryStorage({
    "buzz-communities": "new",
    "buzz-active-community-id": "new",
    "buzz-workspaces": "old",
    "buzz-active-workspace-id": "old",
  });

  clearCommunityStorage(storage);
  migrateLegacyCommunityStorage(storage);

  assert.equal(storage.length, 0);
});

test("local-only launch drops remote communities and activates localhost", () => {
  const result = ensureLocalCommunityActive({
    defaultRelayUrl: "ws://localhost:3000",
    identityPubkey: "local-pubkey",
    communities: [
      {
        id: "huhao",
        name: "huhao",
        relayUrl: "wss://huhao.communities.buzz.xyz",
        addedAt: "2026-08-08T00:00:00.000Z",
      },
      {
        id: "local",
        name: "Local Dev",
        relayUrl: "ws://127.0.0.1:3000",
        addedAt: "2026-08-08T00:00:00.000Z",
      },
    ],
    activeId: "huhao",
  });

  assert.equal(result.changed, true);
  assert.equal(result.activeId, "local");
  assert.deepEqual(result.communities, [
    {
      id: "local",
      name: "Local Dev",
      relayUrl: "ws://localhost:3000",
      addedAt: "2026-08-08T00:00:00.000Z",
    },
  ]);
});

test("remote default preserves remote community choices", () => {
  const communities = [
    {
      id: "remote",
      name: "Remote",
      relayUrl: "wss://community.example.com",
      addedAt: "2026-08-08T00:00:00.000Z",
    },
  ];
  const result = ensureLocalCommunityActive({
    defaultRelayUrl: "wss://community.example.com",
    identityPubkey: "remote-pubkey",
    communities,
    activeId: "remote",
  });

  assert.equal(result.changed, false);
  assert.equal(result.activeId, "remote");
  assert.equal(result.communities, communities);
});
