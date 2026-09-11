# Testing Hub Marketplace Server

Cloud catalog and artifact API for WiParse Testing Hub plugins.  
Paired with in-repo client: `test-tools/marketplace.mjs` and `wiparse_core::marketplace`.

## Run

```bash
export MARKETPLACE_PUBLISH_TOKENS=dev-token
node src/index.mjs --port 8787 --data ./data
```

## API

| Method | Path | Auth |
|--------|------|------|
| GET | `/v1/health` | no |
| GET | `/v1/catalog` | no |
| GET | `/v1/plugins/:id` | no |
| GET | `/v1/plugins/:id/versions/:version` | no |
| GET | `/v1/plugins/:id/versions/:version/download` | no |
| POST | `/v1/plugins/:id/versions` | Bearer |
| DELETE | `/v1/plugins/:id/versions/:version` | Bearer |

Publish body:

```json
{
  "meta": { "id": "...", "version": "...", "sha256": "...", "size": 0, "publisher": "wiparse", "channel": "stable" },
  "artifact_base64": "<zip>"
}
```

## Local simulation (no cloud)

From repo root:

```bash
CLEAN=1 node scripts/local-marketplace-sim.mjs
```

This boots loopback HTTP, publishes the demo fixture, pulls into a temp install
root, and checks runner discovery. Set `WIPARSE_MARKETPLACE_ALLOW_HTTP=1` when
calling the client CLI against `http://127.0.0.1:…`.

## Tests

```bash
node --test test/*.test.mjs
```
