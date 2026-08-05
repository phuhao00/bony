# Block/Buzz integration (bony)

- **Submodule**: [`../../third_party/buzz`](../../third_party/buzz) → https://github.com/phuhao00/buzz  
  (see [`.gitmodules`](../../.gitmodules), [`BONY.md`](../../third_party/buzz/BONY.md))
- **Patches** (apply after submodule init): [`patches/`](./patches/)
- **Room automation**: [`../../scripts/buzz-room`](../../scripts/buzz-room)
- **Docs**: [`../../docs/buzz-room-collab.md`](../../docs/buzz-room-collab.md)

```powershell
git submodule update --init --recursive
powershell -File scripts/buzz-room/setup-buzz.ps1
```
