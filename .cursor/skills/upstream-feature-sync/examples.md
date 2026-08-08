# Examples

## Monitor only

User: 「监测一下 upstream grok-build」

Agent: fetch + left-right count + report table; **no** rebase.

## Full monorepo rebase

User: 「帮我 rebase xai-org/grok-build，别把 Buzz 弄坏」

Agent:

1. Ensure clean tree; commit local product/docs first if needed  
2. `git fetch upstream main` + backup tag  
3. `git rebase upstream/main`  
4. `Cargo.lock` conflict → `cargo generate-lockfile`  
5. Keep `third_party/buzz/**`, `bony-*`, room start scripts  
6. `SOURCE_REV` = `upstream/main`  
7. `cargo check -p buzz-desktop -p buzz-relay -p buzz-db`
8. Push only if user asks: `git push --force-with-lease origin main`

## Wrong outcome (do not ship)

- Room again **requires** Docker Compose / Postgres / Redis for local single-instance  
- `third_party/buzz` becomes empty submodule pointer  
- Root `Cargo.toml` dropped `bony-build` or buzz crate members  
- Lost `zeroclaw_weather_tool.rs` overlay  
