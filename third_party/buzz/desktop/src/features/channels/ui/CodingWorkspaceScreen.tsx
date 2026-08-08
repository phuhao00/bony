import * as React from "react";
import {
  ArrowLeft,
  Bot,
  CheckCircle2,
  CircleStop,
  FolderGit2,
  FolderOpen,
  GitBranch,
  LoaderCircle,
  Plus,
  X,
} from "lucide-react";
import { toast } from "sonner";

import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import { useStartManagedAgentMutation } from "@/features/agents/hooks";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import { cancelManagedAgentTurn } from "@/shared/api/agentControl";
import {
  forgetCodingWorkspaceProject,
  listCodingWorkspaceProjects,
  openCodingWorkspaceProject,
  type CodingWorkspaceProject,
} from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Badge } from "@/shared/ui/badge";
import type { ChannelAgentSessionAgent } from "./useChannelAgentSessions";
import {
  codingAgentRuntimeLabel,
  selectCodingWorkspaceAgent,
  withSelectedCodingAgentMention,
} from "./codingWorkspaceSurface";

type CodingWorkspaceScreenProps = {
  agents: ChannelAgentSessionAgent[];
  channel: Channel;
  isSending: boolean;
  onClose: () => void;
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

  return (
    <div className="flex min-h-0 min-w-0 flex-1 border-t border-border/45">
      <aside className="flex w-56 shrink-0 flex-col border-r border-border/55 bg-muted/20">
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
            </div>
            {project.gitBranch ? (
              <div className="mt-1.5 flex items-center gap-1.5 text-xs text-muted-foreground">
                <GitBranch className="h-3.5 w-3.5" />
                <span className="truncate">{project.gitBranch}</span>
              </div>
            ) : (
              <p className="mt-1.5 text-xs text-muted-foreground">
                Local folder
              </p>
            )}
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-3">
          <div className="mb-2 flex items-center justify-between px-1">
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              Coding agents
            </span>
            <Badge variant="secondary">{agents.length}</Badge>
          </div>
          <div className="space-y-1">
            {agents.map((agent) => {
              const selected = selectedAgent?.pubkey === agent.pubkey;
              const profile = profiles?.[normalizePubkey(agent.pubkey)] ?? null;
              const label = resolveUserLabel({
                pubkey: agent.pubkey,
                fallbackName: agent.name,
                profiles,
                preferResolvedSelfLabel: true,
              });
              return (
                <button
                  className={cn(
                    "group flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left transition-[background-color,color,transform] duration-150",
                    selected
                      ? "bg-accent text-accent-foreground shadow-xs"
                      : "text-muted-foreground hover:translate-x-0.5 hover:bg-accent/55 hover:text-foreground",
                  )}
                  key={agent.pubkey}
                  onClick={() => onSelectAgent(agent.pubkey)}
                  type="button"
                >
                  <ProfileAvatar
                    avatarUrl={profile?.avatarUrl ?? null}
                    className="size-8"
                    label={label}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium">
                      {label}
                    </span>
                    <span className="block truncate text-xs opacity-70">
                      {codingAgentRuntimeLabel(agent)}
                    </span>
                  </span>
                  {selected ? (
                    <CheckCircle2 className="h-4 w-4 shrink-0 text-primary" />
                  ) : null}
                </button>
              );
            })}
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
                  {working
                    ? `Working in ${project.name}`
                    : `Ready in ${project.name}`}
                </p>
              </div>
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
              <ManagedAgentSessionPanel
                agent={selectedAgent}
                channelId={channel.id}
                className="mx-auto max-w-5xl border-0 bg-transparent shadow-none"
                emptyDescription={`Describe a task below. ${selectedAgent.name} will work inside ${project.path}.`}
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
                placeholder={`Ask ${selectedAgent.name} to change this project…`}
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
                Deploy an ACP agent to this room. Grok, Codex, Claude Code, and
                custom runtimes will appear here through the same managed-agent
                catalog.
              </p>
            </div>
          </div>
        )}
      </main>
    </div>
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
