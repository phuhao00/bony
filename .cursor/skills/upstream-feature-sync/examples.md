# Examples

## Example 1: User — 「帮我监测 ZeroClaw 和 OpenMontage 有没有更新」

Agent:

1. Reads this skill.
2. Fetches only (no rebase).
3. Outputs the markdown table under “Report template”.
4. Stops until user says to sync.

## Example 2: User — 「rebase xai-org/grok-build:main」

Agent:

1. Clean tree check → `git fetch upstream main` → `git rebase upstream/main`.
2. On `Cargo.lock` conflict: regenerate lock, continue.
3. Update `SOURCE_REV` to `upstream/main`.
4. `cargo check -p bony-build`.
5. Mentions force-with-lease needed for origin; does not push unless asked.

## Example 3: User — 「把 ZeroClaw 拉到最新，别丢我们的天气补丁」

Agent:

1. `git pull` from `origin` (`zeroclaw-labs/zeroclaw`, branch `master`) in `~/.bony-build/zeroclaw`.
2. Ensures weather overlay is applied (content matches asset or rebuild triggers patch).
3. `cargo +stable build --release --bin zeroclaw`.
4. Smoke notes: weather location for 深圳 / native_tools / agentic.

## Example 4: After long monorepo rebase OpenMontage install broken

Agent:

1. Does **not** wipe OpenMontage skill logic.
2. Diffs only `openmontage.rs` vs pre-rebase if needed.
3. Re-runs local OpenMontage deps install; keeps `GITHUB_URL` and skill prompt shape.
