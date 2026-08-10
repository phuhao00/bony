import * as React from "react";
import {
  ArrowLeft,
  Bot,
  CheckCircle2,
  ChevronRight,
  CircleStop,
  File,
  FileDiff,
  Files,
  Folder,
  FolderGit2,
  FolderOpen,
  GitBranch,
  GitCommitHorizontal,
  History,
  LoaderCircle,
  Plus,
  RefreshCw,
  Settings2,
  Wrench,
  X,
} from "lucide-react";
import { toast } from "sonner";

import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import { useStartManagedAgentMutation } from "@/features/agents/hooks";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import { ProjectFileDiffPreview } from "@/features/projects/ui/ProjectPullRequestFilesChangedPanel";
import { cancelManagedAgentTurn } from "@/shared/api/agentControl";
import {
  forgetCodingWorkspaceProject,
  getCodingWorkspaceFileDiff,
  getCodingWorkspaceSnapshot,
  listCodingWorkspaceProjects,
  openCodingWorkspaceProject,
  type CodingWorkspaceChange,
  type CodingWorkspaceFileDiff,
  type CodingWorkspaceProject,
  type CodingWorkspaceSnapshot,
} from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { cn } from "@/shared/lib/cn";
import {
  buildPathTree,
  type PathTreeItem,
  type PathTreeNode,
  sortedPathTreeChildren,
} from "@/shared/lib/pathTree";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Badge } from "@/shared/ui/badge";
import type { ChannelAgentSessionAgent } from "./useChannelAgentSessions";
import {
  codingAgentRuntimeLabel,
  codingWorkspaceAgentRoleLabel,
  isCodingWorkspaceCodingAgent,
  selectCodingWorkspaceAgent,
  withSelectedCodingAgentMention,
} from "./codingWorkspaceSurface";

type CodingWorkspaceScreenProps = {
  agents: ChannelAgentSessionAgent[];
  channel: Channel;
  isSending: boolean;
  onClose: () => void;
  onEditAgent: (pubkey: string) => void;
  onSendPrompt: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    channelId?: string | null,
    codingWorkspacePath?: string,
  ) => Promise<void>;
  profiles?: UserProfileLookup;
};

export function CodingWorkspaceScreen({
  agents,
  channel,
  isSending,
  onClose,
  onEditAgent,
  onSendPrompt,
  profiles,
}: CodingWorkspaceScreenProps) {
  const codingAgents = React.useMemo(
    () => agents.filter((agent) => agent.agentSource === "managed"),
    [agents],
  );
  const [recentProjects, setRecentProjects] = React.useState<
    CodingWorkspaceProject[]
  >([]);
  const [activeProject, setActiveProject] =
    React.useState<CodingWorkspaceProject | null>(null);
  const [selectedAgentPubkey, setSelectedAgentPubkey] = React.useState<
    string | null
  >(null);
  const [projectsPending, setProjectsPending] = React.useState(true);
  const [pickerPending, setPickerPending] = React.useState(false);

  const selectedAgent = React.useMemo(
    () => selectCodingWorkspaceAgent(codingAgents, selectedAgentPubkey),
    [codingAgents, selectedAgentPubkey],
  );

  React.useEffect(() => {
    let cancelled = false;
    void listCodingWorkspaceProjects()
      .then((projects) => {
        if (!cancelled) setRecentProjects(projects);
      })
      .catch((error) => {
        if (!cancelled) {
          toast.error(
            error instanceof Error
              ? error.message
              : "Failed to load recent projects.",
          );
        }
      })
      .finally(() => {
        if (!cancelled) setProjectsPending(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const openProject = React.useCallback(async (path?: string) => {
    setPickerPending(true);
    try {
      const project = await openCodingWorkspaceProject(path);
      if (!project) return;
      setActiveProject(project);
      setRecentProjects((projects) => [
        project,
        ...projects.filter((candidate) => candidate.id !== project.id),
      ]);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to open project.",
      );
    } finally {
      setPickerPending(false);
    }
  }, []);

  const forgetProject = React.useCallback(
    async (project: CodingWorkspaceProject) => {
      try {
        await forgetCodingWorkspaceProject(project.path);
        setRecentProjects((projects) =>
          projects.filter((candidate) => candidate.id !== project.id),
        );
      } catch (error) {
        toast.error(
          error instanceof Error
            ? error.message
            : "Failed to remove recent project.",
        );
      }
    },
    [],
  );

  return (
    <div
      className="flex h-full min-h-0 min-w-0 flex-1 flex-col bg-background text-foreground"
      data-testid="coding-workspace"
    >
      <ChatHeader
        actions={
          <div className="flex items-center gap-1">
            {activeProject ? (
              <Button
                aria-label="Choose another project"
                onClick={() => void openProject()}
                size="sm"
                type="button"
                variant="outline"
              >
                <FolderOpen className="mr-1.5 h-4 w-4" />
                Switch project
              </Button>
            ) : null}
            <Button
              aria-label="Return to room"
              data-testid="close-coding-workspace"
              onClick={onClose}
              size="icon"
              title="Return to room"
              type="button"
              variant="ghost"
            >
              <X />
            </Button>
          </div>
        }
        belowSystemChrome
        description={
          activeProject
            ? activeProject.path
            : "Open a local project and assign a coding agent"
        }
        mode="projects"
        title={activeProject?.name ?? "Coding Workspace"}
      />

      {activeProject ? (
        <ActiveCodingWorkspace
          agents={codingAgents}
          channel={channel}
          isSending={isSending}
          onBack={() => setActiveProject(null)}
          onEditAgent={onEditAgent}
          onSelectAgent={setSelectedAgentPubkey}
          onSendPrompt={onSendPrompt}
          profiles={profiles}
          project={activeProject}
          selectedAgent={selectedAgent}
        />
      ) : (
        <ProjectPicker
          onForgetProject={forgetProject}
          onOpenProject={openProject}
          pending={pickerPending || projectsPending}
          projects={recentProjects}
        />
      )}
    </div>
  );
}

type ActiveCodingWorkspaceProps = {
  agents: ChannelAgentSessionAgent[];
  channel: Channel;
  isSending: boolean;
  onBack: () => void;
  onEditAgent: (pubkey: string) => void;
  onSelectAgent: (pubkey: string) => void;
  onSendPrompt: CodingWorkspaceScreenProps["onSendPrompt"];
  profiles?: UserProfileLookup;
  project: CodingWorkspaceProject;
  selectedAgent: ChannelAgentSessionAgent | null;
};

function ActiveCodingWorkspace({
  agents,
  channel,
  isSending,
  onBack,
  onEditAgent,
  onSelectAgent,
  onSendPrompt,
  profiles,
  project,
  selectedAgent,
}: ActiveCodingWorkspaceProps) {
  const startAgentMutation = useStartManagedAgentMutation();
  const { working } = useAgentWorking(
    selectedAgent?.pubkey ?? null,
    channel.id,
  );
  const [snapshot, setSnapshot] =
    React.useState<CodingWorkspaceSnapshot | null>(null);
  const [snapshotPending, setSnapshotPending] = React.useState(true);
  const [snapshotError, setSnapshotError] = React.useState<string | null>(null);
  const [workspaceView, setWorkspaceView] = React.useState<"files" | "source">(
    "files",
  );
  const [reviewPath, setReviewPath] = React.useState<string | null>(null);
  const snapshotRequestId = React.useRef(0);

  const loadSnapshot = React.useCallback(async () => {
    const requestId = ++snapshotRequestId.current;
    setSnapshotPending(true);
    setSnapshotError(null);
    try {
      const nextSnapshot = await getCodingWorkspaceSnapshot(project.path);
      if (snapshotRequestId.current === requestId) {
        setSnapshot(nextSnapshot);
      }
    } catch (error) {
      if (snapshotRequestId.current === requestId) {
        setSnapshotError(
          error instanceof Error ? error.message : "Failed to inspect project.",
        );
      }
    } finally {
      if (snapshotRequestId.current === requestId) {
        setSnapshotPending(false);
      }
    }
  }, [project.path]);

  React.useEffect(() => {
    void loadSnapshot();
  }, [loadSnapshot]);

  const previousWorking = React.useRef(working);
  React.useEffect(() => {
    if (previousWorking.current && !working) {
      void loadSnapshot();
    }
    previousWorking.current = working;
  }, [loadSnapshot, working]);

  const handleSend = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
      capturedChannelId?: string | null,
    ) => {
      if (!selectedAgent) return;
      if (!isManagedAgentActive(selectedAgent)) {
        await startAgentMutation.mutateAsync(selectedAgent.pubkey);
      }
      await onSendPrompt(
        content,
        withSelectedCodingAgentMention(
          mentionPubkeys,
          selectedAgent.pubkey,
          agents.map((agent) => agent.pubkey),
        ),
        mediaTags,
        capturedChannelId,
        project.path,
      );
    },
    [
      agents,
      onSendPrompt,
      project.path,
      selectedAgent,
      startAgentMutation.mutateAsync,
    ],
  );

  const stopTurn = React.useCallback(async () => {
    if (!selectedAgent) return;
    try {
      await cancelManagedAgentTurn(selectedAgent.pubkey, channel.id);
      toast.success(`Stop signal sent to ${selectedAgent.name}.`);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to stop current turn.",
      );
    }
  }, [channel.id, selectedAgent]);

  const codingAgents = agents.filter(isCodingWorkspaceCodingAgent);
  const specialistAgents = agents.filter(
    (agent) => !isCodingWorkspaceCodingAgent(agent),
  );

  return (
    <div className="flex min-h-0 min-w-0 flex-1 border-t border-border/45">
      <WorkspaceExplorer
        error={snapshotError}
        onBack={onBack}
        onRefresh={() => void loadSnapshot()}
        onSelectChange={(path) => {
          setWorkspaceView("source");
          setReviewPath(path);
        }}
        onViewChange={setWorkspaceView}
        pending={snapshotPending}
        project={project}
        snapshot={snapshot}
        selectedChangePath={reviewPath}
        view={workspaceView}
      />
      {reviewPath ? (
        <CodingWorkspaceReview
          filePath={reviewPath}
          onClose={() => setReviewPath(null)}
          project={project}
        />
      ) : (
        <>
          <aside className="flex w-56 shrink-0 flex-col border-r border-border/55 bg-muted/20">
            <div className="min-h-0 flex-1 overflow-y-auto p-3">
              <div className="mb-2 flex items-center justify-between px-1">
                <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  Project agents
                </span>
                <Badge variant="secondary">{agents.length}</Badge>
              </div>
              <div className="space-y-4">
                <AgentGroup
                  agents={codingAgents}
                  label="Coding"
                  onEditAgent={onEditAgent}
                  onSelectAgent={onSelectAgent}
                  profiles={profiles}
                  selectedPubkey={selectedAgent?.pubkey ?? null}
                />
                <AgentGroup
                  agents={specialistAgents}
                  label="Specialists"
                  onEditAgent={onEditAgent}
                  onSelectAgent={onSelectAgent}
                  profiles={profiles}
                  selectedPubkey={selectedAgent?.pubkey ?? null}
                />
              </div>
            </div>
          </aside>

          <main className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
            {selectedAgent ? (
              <>
                <div className="flex h-12 shrink-0 items-center gap-3 border-b border-border/45 px-5">
                  <Bot className="h-4 w-4 text-muted-foreground" />
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">
                      {selectedAgent.name}
                    </p>
                    <p className="truncate text-xs text-muted-foreground">
                      {working ? "Working" : "Ready"} ·{" "}
                      {codingWorkspaceAgentRoleLabel(selectedAgent)}
                    </p>
                  </div>
                  <Badge className="max-w-52 truncate" variant="outline">
                    {selectedAgent.model?.trim() || "Runtime default model"}
                  </Badge>
                  {selectedAgent.provider ? (
                    <Badge variant="secondary">{selectedAgent.provider}</Badge>
                  ) : null}
                  <Button
                    aria-label={`Edit ${selectedAgent.name} configuration`}
                    onClick={() => onEditAgent(selectedAgent.pubkey)}
                    size="icon"
                    title="Edit agent configuration"
                    type="button"
                    variant="ghost"
                  >
                    <Settings2 className="h-4 w-4" />
                  </Button>
                  {working ? (
                    <Button
                      onClick={() => void stopTurn()}
                      size="sm"
                      type="button"
                      variant="outline"
                    >
                      <CircleStop className="mr-1.5 h-4 w-4" />
                      Stop
                    </Button>
                  ) : null}
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
                  <div className="mx-auto mb-2 flex max-w-5xl items-center justify-between px-3 text-xs text-muted-foreground">
                    <span className="font-medium text-foreground">
                      Conversation &amp; process
                    </span>
                    <span>Messages · plans · tools · model usage</span>
                  </div>
                  <ManagedAgentSessionPanel
                    agent={selectedAgent}
                    channelId={channel.id}
                    className="mx-auto max-w-5xl border-0 bg-transparent shadow-none"
                    emptyDescription={
                      isCodingWorkspaceCodingAgent(selectedAgent)
                        ? `Describe a coding task below. ${selectedAgent.name} will work inside ${project.path}.`
                        : `Ask ${selectedAgent.name} for its specialist work on ${project.name}.`
                    }
                    panelPadding={false}
                    profiles={profiles}
                    rawLayout="exclusive"
                    showHeader={false}
                    showRaw={false}
                    transcriptContentClassName="pb-8"
                  />
                </div>
                <div className="shrink-0 border-t border-border/45 bg-background/90 pt-2 backdrop-blur-xl">
                  <div className="flex items-center gap-2 px-5 pb-1 text-xs text-muted-foreground">
                    <FolderGit2 className="h-3.5 w-3.5" />
                    <span className="truncate">{project.path}</span>
                  </div>
                  <MessageComposer
                    channelId={channel.id}
                    channelName={channel.name}
                    channelType={channel.channelType}
                    containerClassName="px-4 pb-4"
                    disabled={!channel.isMember || Boolean(channel.archivedAt)}
                    draftKey={`coding-workspace:${project.id}:${selectedAgent.pubkey}`}
                    isSending={isSending || startAgentMutation.isPending}
                    onSend={handleSend}
                    placeholder={
                      isCodingWorkspaceCodingAgent(selectedAgent)
                        ? `Ask ${selectedAgent.name} to change this project…`
                        : `Ask ${selectedAgent.name} about this project…`
                    }
                    profiles={profiles}
                    showBackgroundUploadProgress={false}
                  />
                </div>
              </>
            ) : (
              <div className="flex flex-1 items-center justify-center p-8 text-center">
                <div className="max-w-md">
                  <Bot className="mx-auto h-9 w-9 text-muted-foreground" />
                  <h2 className="mt-4 text-base font-semibold">
                    Add a local coding agent
                  </h2>
                  <p className="mt-2 text-sm text-muted-foreground">
                    Deploy an ACP agent to this room. Grok, Codex, Claude Code,
                    and custom runtimes will appear here through the same
                    managed-agent catalog.
                  </p>
                </div>
              </div>
            )}
          </main>
        </>
      )}
    </div>
  );
}

type AgentGroupProps = {
  agents: ChannelAgentSessionAgent[];
  label: string;
  onEditAgent: (pubkey: string) => void;
  onSelectAgent: (pubkey: string) => void;
  profiles?: UserProfileLookup;
  selectedPubkey: string | null;
};

function AgentGroup({
  agents,
  label,
  onEditAgent,
  onSelectAgent,
  profiles,
  selectedPubkey,
}: AgentGroupProps) {
  return (
    <section>
      <div className="mb-1.5 flex items-center gap-1.5 px-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        {label === "Coding" ? (
          <Bot className="h-3.5 w-3.5" />
        ) : (
          <Wrench className="h-3.5 w-3.5" />
        )}
        <span>{label}</span>
        <span className="ml-auto tabular-nums">{agents.length}</span>
      </div>
      <div className="space-y-1">
        {agents.length === 0 ? (
          <p className="rounded-md border border-dashed border-border/70 px-2 py-3 text-center text-xs text-muted-foreground">
            No {label.toLowerCase()} agents
          </p>
        ) : (
          agents.map((agent) => {
            const selected = selectedPubkey === agent.pubkey;
            const profile = profiles?.[normalizePubkey(agent.pubkey)] ?? null;
            const agentLabel = resolveUserLabel({
              pubkey: agent.pubkey,
              fallbackName: agent.name,
              profiles,
              preferResolvedSelfLabel: true,
            });
            return (
              <div
                className={cn(
                  "group flex items-center rounded-lg transition-[background-color,color,transform] duration-150",
                  selected
                    ? "bg-accent text-accent-foreground shadow-xs"
                    : "text-muted-foreground hover:translate-x-0.5 hover:bg-accent/55 hover:text-foreground",
                )}
                key={agent.pubkey}
              >
                <button
                  className="flex min-w-0 flex-1 items-center gap-2.5 px-2.5 py-2 text-left"
                  onClick={() => onSelectAgent(agent.pubkey)}
                  title={
                    agent.capabilities.join(", ") || "No declared capability"
                  }
                  type="button"
                >
                  <ProfileAvatar
                    avatarUrl={profile?.avatarUrl ?? null}
                    className="size-8"
                    label={agentLabel}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium">
                      {agentLabel}
                    </span>
                    <span className="block truncate text-xs opacity-70">
                      {codingWorkspaceAgentRoleLabel(agent)} ·{" "}
                      {codingAgentRuntimeLabel(agent)}
                    </span>
                  </span>
                  {selected ? (
                    <CheckCircle2 className="h-4 w-4 shrink-0 text-primary" />
                  ) : null}
                </button>
                <Button
                  aria-label={`Edit ${agentLabel} configuration`}
                  className="mr-1 size-7 shrink-0 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
                  onClick={() => onEditAgent(agent.pubkey)}
                  size="icon"
                  title="Edit agent configuration"
                  type="button"
                  variant="ghost"
                >
                  <Settings2 className="h-3.5 w-3.5" />
                </Button>
              </div>
            );
          })
        )}
      </div>
    </section>
  );
}

function CodingWorkspaceReview({
  filePath,
  onClose,
  project,
}: {
  filePath: string;
  onClose: () => void;
  project: CodingWorkspaceProject;
}) {
  const [diff, setDiff] = React.useState<CodingWorkspaceFileDiff | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [pending, setPending] = React.useState(true);
  const requestId = React.useRef(0);

  const loadDiff = React.useCallback(async () => {
    const currentRequest = ++requestId.current;
    setPending(true);
    setError(null);
    setDiff(null);
    try {
      const nextDiff = await getCodingWorkspaceFileDiff(project.path, filePath);
      if (requestId.current === currentRequest) {
        setDiff(nextDiff);
      }
    } catch (reason) {
      if (requestId.current === currentRequest) {
        setError(
          reason instanceof Error
            ? reason.message
            : typeof reason === "string"
              ? reason
              : "Failed to load this change.",
        );
      }
    } finally {
      if (requestId.current === currentRequest) {
        setPending(false);
      }
    }
  }, [filePath, project.path]);

  React.useEffect(() => {
    void loadDiff();
  }, [loadDiff]);

  return (
    <main className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
      <div className="flex min-h-12 shrink-0 items-center gap-3 border-b border-border/45 px-4">
        <Button
          className="h-8"
          onClick={onClose}
          size="sm"
          type="button"
          variant="ghost"
        >
          <ArrowLeft className="mr-1.5 h-4 w-4" />
          Conversation
        </Button>
        <div className="h-5 w-px bg-border" />
        <FileDiff className="h-4 w-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Review
          </p>
          <p className="truncate text-sm" title={filePath}>
            {filePath}
          </p>
        </div>
        {diff ? (
          <div className="flex items-center gap-3 text-xs">
            <span className="text-green-600 dark:text-green-400">
              +{diff.additions}
            </span>
            <span className="text-destructive">-{diff.deletions}</span>
          </div>
        ) : null}
        <Button
          aria-label="Refresh file diff"
          disabled={pending}
          onClick={() => void loadDiff()}
          size="icon"
          title="Refresh diff"
          type="button"
          variant="ghost"
        >
          <RefreshCw className={cn("h-4 w-4", pending && "animate-spin")} />
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto bg-muted/10 p-4">
        {pending ? (
          <div className="flex h-full min-h-48 items-center justify-center gap-2 text-sm text-muted-foreground">
            <LoaderCircle className="h-4 w-4 animate-spin" />
            Loading change…
          </div>
        ) : error ? (
          <div className="mx-auto max-w-3xl rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm text-destructive">
            {error}
          </div>
        ) : diff ? (
          <article className="mx-auto min-w-[42rem] max-w-6xl overflow-hidden rounded-lg border border-border/60 bg-background shadow-sm">
            <header className="flex min-h-10 items-center justify-between gap-3 border-b border-border/50 bg-muted/20 px-3 text-xs">
              <div className="flex min-w-0 items-center gap-2">
                <FileDiff className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <span className="truncate font-medium">{diff.path}</span>
              </div>
              <div className="flex shrink-0 items-center gap-2">
                <Badge variant="outline">Working tree</Badge>
                <span className="text-green-600 dark:text-green-400">
                  +{diff.additions}
                </span>
                <span className="text-destructive">-{diff.deletions}</span>
              </div>
            </header>
            <ProjectFileDiffPreview file={diff} />
          </article>
        ) : null}
      </div>
    </main>
  );
}

type WorkspaceExplorerProps = {
  error: string | null;
  onBack: () => void;
  onRefresh: () => void;
  onSelectChange: (path: string) => void;
  onViewChange: (view: "files" | "source") => void;
  pending: boolean;
  project: CodingWorkspaceProject;
  selectedChangePath: string | null;
  snapshot: CodingWorkspaceSnapshot | null;
  view: "files" | "source";
};

const CHANGE_LABELS: Record<CodingWorkspaceChange["kind"], string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  copied: "C",
  untracked: "U",
  conflict: "!",
};

function compactFolderNode<T extends PathTreeItem>(
  initialNode: PathTreeNode<T>,
): { label: string; node: PathTreeNode<T> } {
  const names = [initialNode.name];
  let node = initialNode;
  while (node.children.size === 1) {
    const child = node.children.values().next().value as
      | PathTreeNode<T>
      | undefined;
    if (!child || child.item) break;
    names.push(child.name);
    node = child;
  }
  return { label: names.join("/"), node };
}

function expandFolderAncestors(current: Set<string>, filePath: string) {
  const next = new Set(current);
  const segments = filePath.split("/").filter(Boolean);
  segments.pop();
  for (let index = 1; index <= segments.length; index += 1) {
    next.add(segments.slice(0, index).join("/"));
  }
  return next;
}

function toggleFolderPath(current: Set<string>, path: string) {
  const next = new Set(current);
  if (next.has(path)) next.delete(path);
  else next.add(path);
  return next;
}

function WorkspacePathTree<T extends PathTreeItem>({
  depth = 0,
  expandedFolders,
  node,
  onToggleFolder,
  renderItem,
}: {
  depth?: number;
  expandedFolders: ReadonlySet<string>;
  node: PathTreeNode<T>;
  onToggleFolder: (path: string) => void;
  renderItem: (item: T, name: string, depth: number) => React.ReactNode;
}) {
  return sortedPathTreeChildren(node).map((child) => {
    if (child.item) {
      return renderItem(child.item, child.name, depth);
    }

    const compacted = compactFolderNode(child);
    const expanded = expandedFolders.has(compacted.node.path);
    return (
      <div key={child.path}>
        <button
          aria-expanded={expanded}
          className="flex w-full min-w-0 items-center gap-1.5 rounded-md py-1.5 pr-2 text-left text-xs font-medium text-muted-foreground hover:bg-accent/55 hover:text-foreground"
          onClick={() => onToggleFolder(compacted.node.path)}
          style={{ paddingLeft: `${0.35 + depth * 0.85}rem` }}
          title={compacted.node.path}
          type="button"
        >
          <ChevronRight
            className={cn(
              "h-3.5 w-3.5 shrink-0 transition-transform",
              expanded && "rotate-90",
            )}
          />
          {expanded ? (
            <FolderOpen className="h-3.5 w-3.5 shrink-0 text-primary/80" />
          ) : (
            <Folder className="h-3.5 w-3.5 shrink-0 text-primary/70" />
          )}
          <span className="min-w-0 flex-1 truncate">{compacted.label}</span>
          <span className="text-[10px] font-normal tabular-nums opacity-65">
            {compacted.node.itemCount}
          </span>
        </button>
        {expanded ? (
          <WorkspacePathTree
            depth={depth + 1}
            expandedFolders={expandedFolders}
            node={compacted.node}
            onToggleFolder={onToggleFolder}
            renderItem={renderItem}
          />
        ) : null}
      </div>
    );
  });
}

const COMMIT_DATE_FORMATTER = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
});

function formatCommitDate(timestamp: number): string {
  return COMMIT_DATE_FORMATTER.format(new Date(timestamp * 1000));
}

function WorkspaceExplorer({
  error,
  onBack,
  onRefresh,
  onSelectChange,
  onViewChange,
  pending,
  project,
  selectedChangePath,
  snapshot,
  view,
}: WorkspaceExplorerProps) {
  const changesByPath = React.useMemo(
    () =>
      new Map((snapshot?.changes ?? []).map((change) => [change.path, change])),
    [snapshot?.changes],
  );
  const fileTree = React.useMemo(
    () => buildPathTree(snapshot?.files ?? []),
    [snapshot?.files],
  );
  const changeTree = React.useMemo(
    () => buildPathTree(snapshot?.changes ?? []),
    [snapshot?.changes],
  );
  const [expandedFileFolders, setExpandedFileFolders] = React.useState<
    Set<string>
  >(new Set());
  const [expandedChangeFolders, setExpandedChangeFolders] = React.useState<
    Set<string>
  >(new Set());

  React.useEffect(() => {
    if (!selectedChangePath) return;
    setExpandedFileFolders((current) =>
      expandFolderAncestors(current, selectedChangePath),
    );
    setExpandedChangeFolders((current) =>
      expandFolderAncestors(current, selectedChangePath),
    );
  }, [selectedChangePath]);

  return (
    <aside className="flex w-72 shrink-0 flex-col border-r border-border/55 bg-muted/10">
      <div className="border-b border-border/45 px-3 py-3">
        <Button
          className="h-8 w-full justify-start px-2 text-muted-foreground"
          onClick={onBack}
          size="sm"
          type="button"
          variant="ghost"
        >
          <ArrowLeft className="mr-2 h-4 w-4" />
          All projects
        </Button>
        <div className="mt-3 rounded-lg border border-border/60 bg-background/75 px-3 py-2.5 shadow-xs">
          <div className="flex items-center gap-2">
            <FolderGit2 className="h-4 w-4 shrink-0 text-primary" />
            <span className="truncate text-sm font-semibold">
              {project.name}
            </span>
            <Button
              aria-label="Refresh project files and Git state"
              className="ml-auto size-7"
              disabled={pending}
              onClick={onRefresh}
              size="icon"
              title="Refresh"
              type="button"
              variant="ghost"
            >
              <RefreshCw
                className={cn("h-3.5 w-3.5", pending && "animate-spin")}
              />
            </Button>
          </div>
          {snapshot?.isGitRepository || project.gitBranch ? (
            <div className="mt-1.5 flex items-center gap-1.5 text-xs text-muted-foreground">
              <GitBranch className="h-3.5 w-3.5" />
              <span className="truncate">
                {snapshot?.gitBranch ?? project.gitBranch ?? "Git repository"}
              </span>
            </div>
          ) : (
            <p className="mt-1.5 text-xs text-muted-foreground">Local folder</p>
          )}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-1 border-b border-border/45 p-2">
        <Button
          className="h-8 justify-start px-2"
          onClick={() => onViewChange("files")}
          size="sm"
          type="button"
          variant={view === "files" ? "secondary" : "ghost"}
        >
          <Files className="mr-1.5 h-3.5 w-3.5" />
          Files
          <span className="ml-auto text-[11px] tabular-nums">
            {snapshot?.files.length ?? 0}
          </span>
        </Button>
        <Button
          className="h-8 justify-start px-2"
          onClick={() => onViewChange("source")}
          size="sm"
          type="button"
          variant={view === "source" ? "secondary" : "ghost"}
        >
          <GitBranch className="mr-1.5 h-3.5 w-3.5" />
          Changes
          <span className="ml-auto text-[11px] tabular-nums">
            {snapshot?.changes.length ?? 0}
          </span>
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {error ? (
          <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
            {error}
          </div>
        ) : pending && !snapshot ? (
          <div className="flex items-center justify-center gap-2 py-8 text-xs text-muted-foreground">
            <LoaderCircle className="h-4 w-4 animate-spin" />
            Inspecting project…
          </div>
        ) : view === "files" ? (
          <div className="space-y-0.5">
            <WorkspacePathTree
              expandedFolders={expandedFileFolders}
              node={fileTree}
              onToggleFolder={(path) =>
                setExpandedFileFolders((current) =>
                  toggleFolderPath(current, path),
                )
              }
              renderItem={(file, name, depth) => {
                const change = changesByPath.get(file.path);
                return (
                  <button
                    className={cn(
                      "flex w-full min-w-0 items-center gap-1.5 rounded-md py-1.5 pr-2 text-left text-xs",
                      change
                        ? "hover:bg-accent/55"
                        : "cursor-default disabled:opacity-100",
                      selectedChangePath === file.path && "bg-accent",
                    )}
                    disabled={!change}
                    key={file.path}
                    onClick={() => change && onSelectChange(file.path)}
                    style={{ paddingLeft: `${0.35 + depth * 0.85}rem` }}
                    title={file.path}
                    type="button"
                  >
                    <span aria-hidden="true" className="w-3.5 shrink-0" />
                    <File className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                    <span className="min-w-0 flex-1 truncate">{name}</span>
                    {change ? (
                      <span
                        className={cn(
                          "font-mono font-semibold",
                          change.kind === "conflict"
                            ? "text-destructive"
                            : "text-primary",
                        )}
                      >
                        {CHANGE_LABELS[change.kind]}
                      </span>
                    ) : null}
                  </button>
                );
              }}
            />
            {snapshot?.filesTruncated ? (
              <p className="px-2 py-2 text-[11px] text-muted-foreground">
                Showing the first {snapshot.files.length} files.
              </p>
            ) : null}
            {snapshot?.files.length === 0 ? (
              <p className="py-8 text-center text-xs text-muted-foreground">
                This folder has no visible files.
              </p>
            ) : null}
          </div>
        ) : (
          <div className="space-y-5">
            <section>
              <div className="mb-1.5 flex items-center px-2 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                Changes
                <span className="ml-auto tabular-nums">
                  {snapshot?.changes.length ?? 0}
                </span>
              </div>
              <div className="space-y-0.5">
                <WorkspacePathTree
                  expandedFolders={expandedChangeFolders}
                  node={changeTree}
                  onToggleFolder={(path) =>
                    setExpandedChangeFolders((current) =>
                      toggleFolderPath(current, path),
                    )
                  }
                  renderItem={(change, name, depth) => (
                    <button
                      className={cn(
                        "flex w-full min-w-0 items-start gap-1.5 rounded-md py-1.5 pr-2 text-left text-xs hover:bg-accent/55",
                        selectedChangePath === change.path && "bg-accent",
                      )}
                      key={`${change.path}:${change.indexStatus}:${change.worktreeStatus}`}
                      onClick={() => onSelectChange(change.path)}
                      style={{ paddingLeft: `${0.35 + depth * 0.85}rem` }}
                      title={`${change.path} · ${change.kind}${change.staged ? " · staged" : ""}`}
                      type="button"
                    >
                      <span aria-hidden="true" className="w-3.5 shrink-0" />
                      <span
                        className={cn(
                          "mt-0.5 w-3.5 shrink-0 text-center font-mono font-semibold",
                          change.kind === "conflict"
                            ? "text-destructive"
                            : "text-primary",
                        )}
                      >
                        {CHANGE_LABELS[change.kind]}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate">{name}</span>
                        {change.originalPath ? (
                          <span
                            className="block truncate text-[11px] text-muted-foreground"
                            title={change.originalPath}
                          >
                            from {change.originalPath.split("/").pop()}
                          </span>
                        ) : null}
                      </span>
                      {change.staged ? (
                        <span className="text-[10px] text-muted-foreground">
                          S
                        </span>
                      ) : null}
                    </button>
                  )}
                />
                {snapshot?.changesTruncated ? (
                  <p className="px-2 py-2 text-[11px] text-muted-foreground">
                    Showing the first {snapshot.changes.length} changes.
                  </p>
                ) : null}
                {snapshot?.isGitRepository && snapshot.changes.length === 0 ? (
                  <p className="px-2 py-3 text-xs text-muted-foreground">
                    Working tree clean.
                  </p>
                ) : null}
                {snapshot && !snapshot.isGitRepository ? (
                  <p className="px-2 py-3 text-xs text-muted-foreground">
                    Version history appears after Git is initialized.
                  </p>
                ) : null}
              </div>
            </section>

            {snapshot?.isGitRepository ? (
              <section>
                <div className="mb-1.5 flex items-center gap-1.5 px-2 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                  <History className="h-3.5 w-3.5" />
                  History
                </div>
                <div className="space-y-1">
                  {snapshot.commits.map((commit) => (
                    <div
                      className="rounded-md px-2 py-1.5 text-xs hover:bg-accent/55"
                      key={commit.hash}
                      title={commit.hash}
                    >
                      <div className="flex items-start gap-2">
                        <GitCommitHorizontal className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                        <span className="min-w-0 flex-1 truncate">
                          {commit.subject}
                        </span>
                      </div>
                      <div className="mt-1 flex items-center gap-2 pl-5 text-[10px] text-muted-foreground">
                        <span className="font-mono">{commit.shortHash}</span>
                        <span className="min-w-0 flex-1 truncate">
                          {commit.authorName}
                        </span>
                        <span>{formatCommitDate(commit.timestamp)}</span>
                      </div>
                    </div>
                  ))}
                  {snapshot.commits.length === 0 ? (
                    <p className="px-2 py-3 text-xs text-muted-foreground">
                      No commits yet.
                    </p>
                  ) : null}
                </div>
              </section>
            ) : null}
          </div>
        )}
      </div>
    </aside>
  );
}

type ProjectPickerProps = {
  onForgetProject: (project: CodingWorkspaceProject) => Promise<void>;
  onOpenProject: (path?: string) => Promise<void>;
  pending: boolean;
  projects: CodingWorkspaceProject[];
};

function ProjectPicker({
  onForgetProject,
  onOpenProject,
  pending,
  projects,
}: ProjectPickerProps) {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-6 py-10">
      <div className="mx-auto max-w-4xl">
        <div className="flex flex-col items-center text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-2xl border border-border/70 bg-muted/45 shadow-sm">
            <FolderGit2 className="h-7 w-7 text-primary" />
          </div>
          <h1 className="mt-5 text-2xl font-semibold tracking-tight">
            Start from a project
          </h1>
          <p className="mt-2 max-w-xl text-sm leading-6 text-muted-foreground">
            Open a local folder, then assign Grok or any future ACP coding agent
            without leaving Buzz.
          </p>
          <Button
            className="mt-6"
            disabled={pending}
            onClick={() => void onOpenProject()}
            size="lg"
            type="button"
          >
            {pending ? (
              <LoaderCircle className="mr-2 h-4 w-4 animate-spin" />
            ) : (
              <FolderOpen className="mr-2 h-4 w-4" />
            )}
            Open project folder
          </Button>
        </div>

        {projects.length > 0 ? (
          <section className="mt-12">
            <div className="mb-3 flex items-center justify-between">
              <h2 className="text-sm font-semibold">Recent projects</h2>
              <span className="text-xs text-muted-foreground">
                {projects.length} available
              </span>
            </div>
            <div className="grid gap-2 sm:grid-cols-2">
              {projects.map((project) => (
                <div
                  className="group relative rounded-xl border border-border/65 bg-card p-1 transition-[border-color,box-shadow,transform] duration-150 hover:-translate-y-0.5 hover:border-primary/35 hover:shadow-md"
                  key={project.id}
                >
                  <button
                    className="flex w-full items-center gap-3 rounded-lg px-3 py-3 text-left"
                    onClick={() => void onOpenProject(project.path)}
                    type="button"
                  >
                    <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-muted">
                      <FolderGit2 className="h-5 w-5 text-muted-foreground" />
                    </div>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium">
                        {project.name}
                      </span>
                      <span className="mt-0.5 block truncate text-xs text-muted-foreground">
                        {project.gitBranch
                          ? `${project.gitBranch} · ${project.path}`
                          : project.path}
                      </span>
                    </span>
                  </button>
                  <Button
                    aria-label={`Remove ${project.name} from recent projects`}
                    className="absolute right-2 top-2 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
                    onClick={() => void onForgetProject(project)}
                    size="icon"
                    title="Remove from recent projects"
                    type="button"
                    variant="ghost"
                  >
                    <X className="h-4 w-4" />
                  </Button>
                </div>
              ))}
            </div>
          </section>
        ) : !pending ? (
          <div className="mt-12 rounded-xl border border-dashed border-border px-6 py-8 text-center text-sm text-muted-foreground">
            <Plus className="mx-auto mb-2 h-5 w-5" />
            Your recent projects will appear here.
          </div>
        ) : null}
      </div>
    </div>
  );
}
