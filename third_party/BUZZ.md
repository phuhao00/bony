# Buzz submodule (phuhao00/buzz)

| | |
|--|--|
| Path | [`buzz/`](./buzz/) |
| Remote | https://github.com/phuhao00/buzz (fork of [block/buzz](https://github.com/block/buzz)) |
| Init | `git submodule update --init --recursive` |
| Helper | `powershell -File scripts/buzz-room/setup-buzz.ps1` |

Grok ACP runtime (`agent stdio` + desktop catalog) is **committed on this fork**.  
Parent pins the submodule commit (mode `160000`).

```powershell
git ls-tree HEAD third_party/buzz
git submodule update --init --recursive
```
