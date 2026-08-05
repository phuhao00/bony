# Buzz Room Collaboration (Grok lead)

Grok is the **room lead**. Buzz is the **shared office**. Specialists: ZeroClaw, Unity, OpenMontage.

Buzz lives in this monorepo as a **git submodule**: `third_party/buzz` → https://github.com/phuhao00/buzz

## Layout

```
bony-build/
  .gitmodules                    # submodule registry
  third_party/buzz/              # git submodule (Block/Buzz)
  integrations/buzz/patches/     # Grok delta applied onto the submodule
  scripts/buzz-room/             # setup / infra / agents
  crates/.../bony-room-tools-mcp
```

## One-time clone of bony with Buzz

```powershell
git clone --recurse-submodules https://github.com/phuhao00/bony.git
# or after a normal clone:
git submodule update --init --recursive
powershell -File .\scripts\buzz-room\setup-buzz.ps1   # apply Grok patches
```

## Quick start (from bony-build root)

```powershell
powershell -File .\scripts\buzz-room\setup-buzz.ps1
powershell -File .\scripts\buzz-room\start-infra.ps1
powershell -File .\scripts\buzz-room\build-tools.ps1
powershell -File .\scripts\buzz-room\mint-agent-keys.ps1
powershell -File .\scripts\buzz-room\start-relay.ps1
powershell -File .\scripts\buzz-room\start-grok-agent.ps1
```

## Policy

- Grok: `subscribe=all`
- Specialists: `subscribe=mentions`
- Permission: `accept-edits`

## Notes

- Submodule pin is a commit SHA in the parent repo; update with  
  `git -C third_party/buzz fetch` + `git -C third_party/buzz checkout <rev>` + `git add third_party/buzz`.
- Grok patches stay in `integrations/buzz/patches` until upstreamed or forked.
