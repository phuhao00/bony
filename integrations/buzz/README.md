# Block/Buzz integration (bony)

- **In-tree**: [`../../third_party/buzz`](../../third_party/buzz) — ordinary monorepo sources (not a submodule)
- **Workspace**: root [`Cargo.toml`](../../Cargo.toml) members include all Buzz crates + `buzz-desktop`
- **Patches** (legacy, if still needed): [`patches/`](./patches/)
- **Room automation**: [`../../scripts/buzz-room`](../../scripts/buzz-room)
- **Docs**: [`../../docs/buzz-room-collab.md`](../../docs/buzz-room-collab.md)

```powershell
# Single workspace build
cargo build -p buzz-desktop
# Or:
powershell -File scripts/buzz-room/build-desktop.ps1
powershell -File scripts/buzz-room/start-desktop.ps1
```
