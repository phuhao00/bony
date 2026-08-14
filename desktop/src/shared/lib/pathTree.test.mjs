import assert from "node:assert/strict";
import test from "node:test";

import { buildPathTree, sortedPathTreeChildren } from "./pathTree.ts";

test("buildPathTree groups project files by directory and counts descendants", () => {
  const root = buildPathTree([
    { path: "src/features/tree.ts" },
    { path: "src/main.ts" },
    { path: "README.md" },
  ]);

  assert.equal(root.itemCount, 3);
  assert.equal(root.children.get("src")?.itemCount, 2);
  assert.equal(
    root.children.get("src")?.children.get("features")?.children.get("tree.ts")
      ?.item?.path,
    "src/features/tree.ts",
  );
});

test("sortedPathTreeChildren keeps folders before files with natural sorting", () => {
  const root = buildPathTree([
    { path: "file10.txt" },
    { path: "folder/file.txt" },
    { path: "file2.txt" },
  ]);

  assert.deepEqual(
    sortedPathTreeChildren(root).map((node) => node.name),
    ["folder", "file2.txt", "file10.txt"],
  );
});
