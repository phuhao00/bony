# Buzz Docker Compose deployment

This is the single-node/VPS deployment bundle. It is intentionally separate from
the root `docker-compose.yml`, which remains local development infrastructure.

## Quick start

```bash
cd deploy/compose
cp .env.example .env
$EDITOR .env       # replace every CHANGE_ME value
./run.sh start
```

For a public VPS with automatic Let's Encrypt certificates:

```bash
cd deploy/compose
BUZZ_COMPOSE_TLS=true ./run.sh start
```

The bootstrap script should eventually replace manual `.env` editing for normal
users. It is responsible for generating stable secrets and, optionally, an owner
keypair.

## Production notes

- Requires Docker Compose v2.24.4 or newer; the TLS override uses Compose's
  `!reset` tag to remove the direct relay port when Caddy terminates HTTPS.
- Default `BUZZ_IMAGE` tracks `ghcr.io/block/buzz:main` for early testing. Pin it to `ghcr.io/block/buzz:sha-<7>` or a semver release tag for production once available.
- Keep `BUZZ_RELAY_PRIVATE_KEY`, `BUZZ_GIT_HOOK_HMAC_SECRET`, database/Redis,
  and S3 secrets stable across restarts.
- `RELAY_OWNER_PUBKEY` is intentionally not prefixed with `BUZZ_`; it must be a
  64-character hex Nostr pubkey when closed relay mode is enabled.
- `BUZZ_AUTO_MIGRATE` is opt-in. Set `BUZZ_AUTO_MIGRATE=true` or run
  `buzz-admin migrate` before starting the relay when bootstrapping a fresh
  database. Auto-migration requires an image that includes embedded SQLx
  migrations.
- The stack uses Postgres, Redis, a git data volume, and (optionally) MinIO
  because Postgres/Redis/git storage are real Buzz dependencies today. Object
  storage does not require the bundled MinIO — see below.
- Bundled MinIO is opt-in via the `minio` Compose profile; it is not started
  by a plain `./run.sh start`. Enable it with `COMPOSE_PROFILES=minio` in
  `.env`, or run `docker compose --profile minio up -d`.

### Using your own S3-compatible storage

The relay never requires the bundled MinIO to start. Point it at any
S3-compatible endpoint you already run or manage (self-hosted MinIO, AWS S3,
Cloudflare R2, Backblaze B2, ...) by setting in `.env`:

```bash
BUZZ_S3_ENDPOINT=https://your-endpoint.example.com
BUZZ_S3_ACCESS_KEY=...
BUZZ_S3_SECRET_KEY=...
BUZZ_S3_BUCKET=buzz-media
# `path` (default) puts the bucket in the URL path; `virtual` puts it in the
# hostname (required by e.g. AWS S3 and Railway Storage Buckets).
BUZZ_S3_ADDRESSING_STYLE=path
```

Leave `COMPOSE_PROFILES` unset (or without `minio`) and `./run.sh start` will
bring up only `relay`, `postgres`, and `redis` — no MinIO container.

Run `./run.sh backup-hint` for the backup checklist.

## Validation

Before sharing an install link publicly, verify a fresh install with:

```bash
cd deploy/compose
cp .env.example .env
$EDITOR .env
./run.sh config
./run.sh start
curl -fsS "http://127.0.0.1:$(grep -E '^BUZZ_HTTP_PORT=' .env | cut -d= -f2-)/_liveness"
./run.sh status
```
