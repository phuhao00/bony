# Block/Buzz submodule

| | |
|--|--|
| Path | [`buzz/`](./buzz/) |
| Remote | https://github.com/block/buzz |
| Init | `git submodule update --init --recursive` |
| Helper | `powershell -File scripts/buzz-room/setup-buzz.ps1` (also applies Grok patches) |
| Patches | [`../integrations/buzz/patches/`](../integrations/buzz/patches/) |
| Docs | [`../docs/buzz-room-collab.md`](../docs/buzz-room-collab.md) |

Pins are **gitlinks** in the parent repo (mode `160000`), not a full tree import.

```powershell
# show pin
git ls-tree HEAD third_party/buzz
```
