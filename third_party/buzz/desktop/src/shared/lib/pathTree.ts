export type PathTreeItem = {
  path: string;
};

export type PathTreeNode<T extends PathTreeItem> = {
  children: Map<string, PathTreeNode<T>>;
  item: T | null;
  itemCount: number;
  name: string;
  path: string;
};

const PATH_TREE_COLLATOR = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});

function createPathTreeNode<T extends PathTreeItem>(
  name: string,
  path: string,
): PathTreeNode<T> {
  return { children: new Map(), item: null, itemCount: 0, name, path };
}

/** Builds a presentation tree from normalized, project-relative paths. */
export function buildPathTree<T extends PathTreeItem>(
  items: readonly T[],
): PathTreeNode<T> {
  const root = createPathTreeNode<T>("", "");
  for (const item of items) {
    const segments = item.path.split("/").filter(Boolean);
    if (segments.length === 0) continue;

    let node = root;
    node.itemCount += 1;
    segments.forEach((segment, index) => {
      const path = segments.slice(0, index + 1).join("/");
      let child = node.children.get(segment);
      if (!child) {
        child = createPathTreeNode<T>(segment, path);
        node.children.set(segment, child);
      }
      child.itemCount += 1;
      node = child;
    });
    node.item = item;
  }
  return root;
}

/** Directories first, then files, both using stable natural-name sorting. */
export function sortedPathTreeChildren<T extends PathTreeItem>(
  node: PathTreeNode<T>,
): PathTreeNode<T>[] {
  return [...node.children.values()].sort((left, right) => {
    if (Boolean(left.item) !== Boolean(right.item)) {
      return left.item ? 1 : -1;
    }
    return PATH_TREE_COLLATOR.compare(left.name, right.name);
  });
}
