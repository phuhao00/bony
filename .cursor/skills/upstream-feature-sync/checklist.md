# Sync run checklist (printable)

```
Task:
[ ] Working tree clean
[ ] Monitor A: fetch upstream/main, note N behind
[ ] Monitor B: fetch zeroclaw managed clone
[ ] Monitor C: fetch OpenMontage install or gh API
[ ] Report table shown to user
[ ] User confirmed apply (if any)

If monorepo rebase:
[ ] git rebase upstream/main complete
[ ] Cargo.lock regenerated if conflicted
[ ] SOURCE_REV = rev-parse upstream/main
[ ] cargo check -p bony-build
[ ] zeroclaw.rs / openmontage.rs / weather asset still present
[ ] push --force-with-lease only if user asked

If ZeroClaw pull:
[ ] weather_tool overlay matches assets/zeroclaw_weather_tool.rs
[ ] release bin rebuilt when patch/binary stale
[ ] config agentic + native_tools + runtime_profile OK

If OpenMontage pull:
[ ] OPENMONTAGE_ROOT pulls cleanly
[ ] skill / OPENMONTAGE_ROOT wiring intact
[ ] UI install status still sensible
```
