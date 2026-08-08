# Sync run checklist (printable)

```
Task:
[ ] Working tree clean (commit README/product WIP first)
[ ] git fetch upstream main
[ ] Note left-right count HEAD...upstream/main
[ ] backup tag: backup/pre-upstream-rebase-YYYYMMDD-HHMMSS
[ ] User confirmed apply (监测 alone is read-only)

Rebase:
[ ] git rebase upstream/main
[ ] Conflicts via decision tree (product/Buzz = keep bony; lockfile = regenerate)
[ ] Cargo.toml members = bony-* + third_party/buzz crates + new upstream members
[ ] Do NOT restore Docker/Postgres/Redis as required room defaults

After:
[ ] SOURCE_REV = git rev-parse upstream/main
[ ] final cargo generate-lockfile if lock was regenerated mid-way
[ ] Path smoke: bony-build zeroclaw/openmontage, buzz-relay, buzz-db, start-room-stack.ps1
[ ] cargo check -p bony-build -p bony-monitor -p buzz-relay -p buzz-db
[ ] push --force-with-lease only if user asked
```
