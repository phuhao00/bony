#![cfg_attr(not(windows), forbid(unsafe_code))]
#![cfg_attr(windows, deny(unsafe_code))]
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData, ServerHandler, ServiceExt,
};
use std::path::Path;
use std::sync::Arc;

mod economy;
mod memory;
mod paths;
mod read_file;
mod rg;
mod route;
mod shell;
mod shim;
mod str_replace;
mod todo;
mod tree;
mod view_image;

#[derive(Clone)]
struct DevMcp {
    state: Arc<shell::SharedState>,
    todos: Arc<todo::TodoState>,
    tool_router: ToolRouter<DevMcp>,
}

#[tool_router]
impl DevMcp {
    fn new(state: Arc<shell::SharedState>) -> Self {
        Self {
            state,
            todos: Arc::new(todo::TodoState::new()),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "shell",
        description = "Run a shell command (bash by default; set `BUZZ_SHELL` to use cmd, PowerShell, or another shell). Ephemeral process per call. Output tail-truncated to ~8KB for the LLM; full output (first 10MB) saved to artifact file. timeout_ms defaults to 120000 (2 min) if omitted; capped at 600000 (10 min). For long-running commands (git push with hooks, cargo build, test suites), use 300000+. On PATH: rg (prefer over grep; flags: -n -i -l -g <glob> -C <n> --files), tree (flags: -d <depth>; shows line counts), and buzz (Buzz relay CLI — run buzz --help for commands)."
    )]
    async fn shell(
        &self,
        Parameters(p): Parameters<shell::ShellParams>,
        context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        shell::run(&self.state, p, context.ct).await
    }

    #[tool(
        name = "read_file",
        description = "Read a text file and return its contents with line numbers. Returns lines in `{number}:{content}` format. Use `offset` (0-based) and `limit` (default 2000) to window into large files. Path resolved relative to workdir (defaults to server cwd). Prefer over cat/head/tail."
    )]
    async fn read_file(
        &self,
        Parameters(p): Parameters<read_file::ReadFileParams>,
    ) -> Result<String, ErrorData> {
        read_file::run(&self.state, p)
    }

    #[tool(
        name = "view_image",
        description = "Load an image from a file path, http(s) URL, or data: URL and return it as an MCP image content block that multimodal LLMs (Anthropic, OpenAI-compatible, etc.) can see. Resizes to a longest-edge of 1568px by default (override with `max_dim`, range 64..=2048). Pass-through for already-small PNG/JPEG; transcodes oversize input to PNG (if alpha) or JPEG q85. Animated GIF/WebP rejected — provide a still frame. Hard cap 20 MiB source, ~4 MiB on the wire. Relative paths resolve under `workdir` (defaults to server cwd) and may not escape it."
    )]
    async fn view_image(
        &self,
        Parameters(p): Parameters<view_image::ViewImageParams>,
    ) -> Result<CallToolResult, ErrorData> {
        view_image::run(&self.state, p).await
    }

    #[tool(
        name = "str_replace",
        description = "Atomic find-and-replace in a file. old_str must occur exactly once unless replace_all is true, in which case all occurrences are replaced. Returns a unified diff. Path resolved relative to workdir (defaults to server cwd). Prefer over sed/awk."
    )]
    async fn str_replace(
        &self,
        Parameters(p): Parameters<str_replace::StrReplaceParams>,
    ) -> Result<String, ErrorData> {
        str_replace::run(&self.state, p)
    }

    #[tool(
        name = "todo",
        description = "Session task list. Omit `todos` to read current state. Provide a full replacement array to update. Items are {text, done}. Open items removed without being marked done will trigger a warning. If the operator enables hooks for this server, the agent's _Stop hook will advise against ending the turn while items are open."
    )]
    async fn todo(
        &self,
        Parameters(p): Parameters<todo::TodoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match self.todos.handle_todo(p) {
            Ok(text) => todo::text_result(text),
            Err(e) => todo::error_result(format!("Error: {e}")),
        }
    }

    /// Hook: called by the agent before honoring end_turn. Returns
    /// non-empty objection text iff items remain open.
    #[tool(
        name = "_Stop",
        description = "Returns open todo items if any exist. Used by the agent's _Stop lifecycle hook to advise against ending with incomplete work."
    )]
    async fn stop_hook(
        &self,
        Parameters(_): Parameters<todo::HookParams>,
    ) -> Result<CallToolResult, ErrorData> {
        todo::text_result(self.todos.stop_objection())
    }

    /// Hook: called by the agent after context compaction/handoff so the
    /// todo list survives history truncation.
    #[tool(
        name = "_PostCompact",
        description = "Internal hook. Agent invokes after handoff; returns todo state for re-injection."
    )]
    async fn post_compact_hook(
        &self,
        Parameters(_): Parameters<todo::HookParams>,
    ) -> Result<CallToolResult, ErrorData> {
        todo::text_result(self.todos.post_compact())
    }

    #[tool(
        name = "memory_append",
        description = "Append one structured summary of a finished room task to the durable task-log (append-only, JSONL, human-readable). Call this once, after the task chain you coordinated is fully delivered — not mid-task. topic should be phrased the way a future similar request would be worded, since it is the primary search key for memory_search."
    )]
    async fn memory_append(
        &self,
        Parameters(p): Parameters<memory::MemoryAppendParams>,
    ) -> Result<String, ErrorData> {
        memory::append(&self.state, p)
    }

    #[tool(
        name = "memory_search",
        description = "Look up past room task-log entries by keyword (case-insensitive substring over topic/notes/agents/outputs) before proposing how to handle a new request, so repeated preferences and known pitfalls carry forward. Returns up to `limit` matches, most recent first."
    )]
    async fn memory_search(
        &self,
        Parameters(p): Parameters<memory::MemorySearchParams>,
    ) -> Result<String, ErrorData> {
        memory::search(&self.state, p)
    }

    #[tool(
        name = "memory_preferences_extract",
        description = "Scan the task-log for notes/feedback that repeat (≥ min_count times). Use occasionally on multi-step or '像上次一样' requests to surface soft format/routing preferences. Never rewrite specialist prompts from this — fold hints into this turn's routing only."
    )]
    async fn memory_preferences_extract(
        &self,
        Parameters(p): Parameters<memory::MemoryPreferencesParams>,
    ) -> Result<String, ErrorData> {
        memory::preferences_extract(&self.state, p)
    }

    #[tool(
        name = "route_list",
        description = "List room agents from the live roster that declare a capability (exact id or namespace prefix like code.). Prefer this over guessing display names when a non-default specialist might own the work. Empty capability lists all declared seats."
    )]
    async fn route_list(
        &self,
        Parameters(p): Parameters<route::RouteListParams>,
    ) -> Result<String, ErrorData> {
        route::list(&self.state, p)
    }

    #[tool(
        name = "route_pick",
        description = "Pick one eligible @Agent for a capability. Order: user preferred_name pin (if eligible) → memory preference_names soft rank → deterministic pubkey/name tie-break. Never invent an agent when this returns none — ask the user or fall back to the fixed ZeroClaw→DocSmith policy pin for research+document."
    )]
    async fn route_pick(
        &self,
        Parameters(p): Parameters<route::RoutePickParams>,
    ) -> Result<String, ErrorData> {
        route::pick(&self.state, p)
    }

    #[tool(
        name = "economy_auction",
        description = "Market path: auction a room task to one running agent using score = 0.5*capability_match + 0.3*reputation + 0.2*stake. Capability mismatch is allowed (money/reputation can override) but marks the contract mismatch=true for heavier failure penalties. Returns contract_id + winner. Prefer route_pick for the default safe capability-hard path; use this when the user asks for bidding/economy or you want risk-priced assignment."
    )]
    async fn economy_auction(
        &self,
        Parameters(p): Parameters<economy::AuctionParams>,
    ) -> Result<String, ErrorData> {
        economy::auction(&self.state, p)
    }

    #[tool(
        name = "economy_subcontract",
        description = "Middleman path: subcontract an awarded economy contract to another agent, taking cut_bp (basis points, default 1000=10%) as immediate brokerage. Depth hard-capped at 2. On descendant failure the broker also takes a reputation hit."
    )]
    async fn economy_subcontract(
        &self,
        Parameters(p): Parameters<economy::SubcontractParams>,
    ) -> Result<String, ErrorData> {
        economy::subcontract(&self.state, p)
    }

    #[tool(
        name = "economy_settle",
        description = "Settle an economy contract as success or failed. Success pays remaining budget + reputation; mismatch success pays bonus rep. Failed+mismatch deducts up to 25% budget (floor 0) and heavy rep; failed+match only small rep hit. Call once at end of the auctioned chain alongside memory_append."
    )]
    async fn economy_settle(
        &self,
        Parameters(p): Parameters<economy::SettleParams>,
    ) -> Result<String, ErrorData> {
        economy::settle(&self.state, p)
    }

    #[tool(
        name = "economy_leaderboard",
        description = "List room agents by reputation then balance (virtual credits + tiers Novice/Adept/Expert/Master/Legend). Use when the user asks for rankings/standings."
    )]
    async fn economy_leaderboard(
        &self,
        Parameters(p): Parameters<economy::LeaderboardParams>,
    ) -> Result<String, ErrorData> {
        economy::leaderboard(&self.state, p)
    }

    #[tool(
        name = "economy_wallet",
        description = "Show one agent's virtual balance, reputation tier, tags, achievements, capability grants, and recent ledger lines. pubkey_or_name accepts display name or pubkey."
    )]
    async fn economy_wallet(
        &self,
        Parameters(p): Parameters<economy::WalletParams>,
    ) -> Result<String, ErrorData> {
        economy::wallet(&self.state, p)
    }

    #[tool(
        name = "economy_org_create",
        description = "Create an agent organization (multi-member economic entity). founder_pubkey is the first member. Org id becomes org:<slug> and can bid on tenders / appear on the leaderboard."
    )]
    async fn economy_org_create(
        &self,
        Parameters(p): Parameters<economy::OrgCreateParams>,
    ) -> Result<String, ErrorData> {
        economy::org_create(&self.state, p)
    }

    #[tool(
        name = "economy_org_join",
        description = "Add a member pubkey to an existing organization (many-to-many: one agent may join multiple orgs)."
    )]
    async fn economy_org_join(
        &self,
        Parameters(p): Parameters<economy::OrgJoinParams>,
    ) -> Result<String, ErrorData> {
        economy::org_join(&self.state, p)
    }

    #[tool(
        name = "economy_org_leave",
        description = "Remove a member pubkey from an organization."
    )]
    async fn economy_org_leave(
        &self,
        Parameters(p): Parameters<economy::OrgLeaveParams>,
    ) -> Result<String, ErrorData> {
        economy::org_leave(&self.state, p)
    }

    #[tool(
        name = "economy_org_list",
        description = "List room organizations with member counts and tags."
    )]
    async fn economy_org_list(
        &self,
        Parameters(p): Parameters<economy::OrgListParams>,
    ) -> Result<String, ErrorData> {
        economy::org_list(&self.state, p)
    }

    #[tool(
        name = "economy_tender_publish",
        description = "Publish an open tender from a title (capability + budget auto-inferred unless overridden). Auto-invites matching agents/orgs; then economy_tender_resolve picks a winner."
    )]
    async fn economy_tender_publish(
        &self,
        Parameters(p): Parameters<economy::TenderPublishParams>,
    ) -> Result<String, ErrorData> {
        economy::tender_publish(&self.state, p)
    }

    #[tool(
        name = "economy_tender_invite",
        description = "Invite capability-matching agents/orgs to bid on an existing open tender (skips parties that already bid)."
    )]
    async fn economy_tender_invite(
        &self,
        Parameters(p): Parameters<economy::TenderResolveParams>,
    ) -> Result<String, ErrorData> {
        economy::tender_invite(&self.state, p)
    }

    #[tool(
        name = "economy_tender_bid",
        description = "Place a bid on an open tender as an agent or org (bidder_kind=agent|org). bidder_pubkey for orgs is org:<slug>."
    )]
    async fn economy_tender_bid(
        &self,
        Parameters(p): Parameters<economy::TenderBidParams>,
    ) -> Result<String, ErrorData> {
        economy::tender_bid(&self.state, p)
    }

    #[tool(
        name = "economy_tender_resolve",
        description = "Resolve an open tender by scoring actual bidders (capability+reputation+stake) and awarding a contract. Prefer this over economy_auction when the market board has open tenders."
    )]
    async fn economy_tender_resolve(
        &self,
        Parameters(p): Parameters<economy::TenderResolveParams>,
    ) -> Result<String, ErrorData> {
        economy::tender_resolve(&self.state, p)
    }

    #[tool(
        name = "economy_tender_list",
        description = "List tenders on the bidding market. Optional status filter: open|resolved|cancelled."
    )]
    async fn economy_tender_list(
        &self,
        Parameters(p): Parameters<economy::TenderListParams>,
    ) -> Result<String, ErrorData> {
        economy::tender_list(&self.state, p)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DevMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "buzz-dev-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(self.state.bootstrap_instructions.clone())
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let argv0 = std::env::args().next().unwrap_or_default();
    let cmd = Path::new(&argv0)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // Multicall dispatch — sync personalities exit before any runtime is built.
    // No tracing, no tokio, no allocations beyond argv parsing.
    match cmd.as_str() {
        "rg" => std::process::exit(rg::run(std::env::args().skip(1).collect())),
        "tree" => std::process::exit(tree::run(std::env::args().skip(1).collect())),
        "git-credential-nostr" => std::process::exit(git_credential_nostr::run()),
        "git-sign-nostr" => std::process::exit(git_sign_nostr::run()),
        _ => {}
    }

    // Async personalities and MCP server mode — build the runtime.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async_main(cmd))
}

async fn async_main(cmd: String) -> Result<(), Box<dyn std::error::Error>> {
    // HTTPS clients invoked through this MCP process need a Rustls provider;
    // repeated installation is harmless.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // buzz CLI needs tokio (async HTTP client).
    if cmd == "buzz" {
        std::process::exit(buzz_cli::run_from_args(std::env::args()).await);
    }

    // MCP server mode — safe to init tracing now.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let cwd = std::env::current_dir()?;
    let shim = shim::Shim::install()?;
    let state = Arc::new(shell::SharedState::new(cwd, shim)?);

    let service = DevMcp::new(state).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

/// Suppress the console window that Windows otherwise allocates for every
/// console-subsystem child process spawned from a non-console parent.
/// No-op on non-Windows platforms.
pub(crate) fn configure_no_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Suppress the console window for async (`tokio::process::Command`) spawns.
/// Equivalent to `configure_no_window` but accepts a tokio command.
/// No-op on non-Windows platforms.
pub(crate) fn configure_no_window_async(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}
