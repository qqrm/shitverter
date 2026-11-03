# Production deployment

The deploy workflow deliberately runs only with **Run workflow**. It publishes
the exact selected commit to GitHub Container Registry (GHCR) and deploys that
immutable image digest over SSH. It must not be changed to deploy every push
until the first production deployment below succeeds on the actual bot VM.

## VM bootstrap

Run these steps once as the account that owns the bot and is allowed to use
Docker. Docker Engine and Docker Compose v2 are required.

```bash
mkdir -p ~/shitverter
chmod 700 ~/shitverter
umask 077
editor ~/shitverter/shitverter.env
```

`shitverter.env` must contain `TELOXIDE_TOKEN` and any non-default limits. Do
not commit it and do not put the Telegram token in GitHub Actions. For example:

```dotenv
TELOXIDE_TOKEN=replace-me
RUST_LOG=info
MAX_INPUT_BYTES=104857600
MAX_CONCURRENT_CONVERSIONS=1
```

Use `chmod 600 ~/shitverter/shitverter.env`. After the repository transfer,
the VM must be allowed to pull `ghcr.io/qqrm-labs/shitverter`. GHCR packages
remain in their original owner's namespace when a repository is transferred.
Publish the new image with **Deploy** and `publish_only=true`, then make the
new package public or authenticate the VM with a classic token scoped to
`read:packages` and access to the package. For a private package:

```bash
printf '%s' "$GHCR_READ_TOKEN" | docker login ghcr.io --username qqrm --password-stdin
```

Create a dedicated SSH key for deployment. Add its public half to the
deployment account's `~/.ssh/authorized_keys`; do not reuse a personal key.

## GitHub environment

Keep the `production` environment in `qqrm-labs/shitverter`, require an approval
reviewer, and add these environment secrets:

| Secret | Value |
| --- | --- |
| `DEPLOY_HOST` | Bot VM hostname or IP address |
| `DEPLOY_USER` | Dedicated deployment account |
| `DEPLOY_PORT` | SSH port, normally `22` |
| `DEPLOY_SSH_PRIVATE_KEY` | Private half of the dedicated deployment key |
| `DEPLOY_KNOWN_HOSTS` | Pinned `known_hosts` line for the VM's SSH host key |

Generate the final value with `ssh-keyscan` only while independently verifying
the displayed fingerprint via the VM provider or console; the workflow itself
never accepts a new host key.

The deploy identity needs Docker access and write access to `~/shitverter`, but
does not need sudo. The workflow copies the deployment script, Compose files,
and local API Dockerfile. It never writes `shitverter.env`, `telegram-api.env`,
or the operator-controlled `local-api.enabled` marker.

## First deployment and rollback

Once the VM can pull the new image, run **Deploy** with `publish_only=false`
from the `main` commit. The workflow builds and pushes a
linux/amd64 image, then passes its SHA-256 image digest to the VM. The remote
script accepts only the old and new GHCR namespaces, verifies that the container is
still running for fifteen seconds, and restores the previous immutable image if
the replacement exits.

After a successful first deployment, confirm the bot accepts a small video and
then decide whether to add `push: branches: [main]` to
`.github/workflows/deploy.yml`. Keeping manual promotion is safer for a bot
without an HTTP health endpoint.

## Large files with the local Telegram Bot API

Telegram's [cloud Bot API](https://core.telegram.org/bots/api#getfile) accepts
metadata for large attachments but only lets bots download files up to 20 MB.
The [official local Bot API server](https://github.com/tdlib/telegram-bot-api#usage)
in `--local` mode removes that download limit and allows uploads up to 2000 MB.
It returns an absolute local file path. The bot therefore needs access to the
same filesystem, not just a different API URL.

The optional deployment uses a separate `shitverter-telegram` Compose project.
It builds the official server from a pinned commit, runs it as UID/GID 1000,
and exposes port 8081 only to its Docker network. No host port is published.
Normal bot deployments neither recreate this server nor remove its session data.
The bot mounts the API data volume read-only at the same absolute path and
copies inputs to its own disk-backed work directory. Both volumes are initialized
by the API image with owner 1000:1000. `TMPDIR` keeps large conversions off the
default 1 GiB RAM-backed `/tmp` mount.
The work volume belongs to exactly one bot container; do not share it with
parallel bot instances or queue workers. Before starting the bot, its local-mode
entrypoint removes its own immediate `shitverter-*` working directories left by
interrupted jobs. Other names, files, and symlinks are preserved. This covers
container restarts after a crash, when normal temporary-directory cleanup cannot
run, and does not touch the separate API cache/session volume.

### One-time setup and migration

1. Deploy the new Shitverter image in the existing cloud mode first and verify
   a small video. Keep its immutable image reference for the activation command
   below. The optional mode is inactive until the marker is created.
2. Obtain your own `api_id` and `api_hash` from
   [my.telegram.org](https://core.telegram.org/api/obtaining_api_id). These are
   application credentials for the API server; the bot keeps its existing token.
   In `~/shitverter`, create `telegram-api.env` with mode 600:

   ```dotenv
   TELEGRAM_API_ID=replace-with-your-numeric-api-id
   TELEGRAM_API_HASH=replace-with-your-api-hash
   ```

3. Build and start the server. This step does not log in a bot until a request
   containing its bot token reaches the server. Compilation is a one-time C++
   build and can take several minutes.

   ```bash
   cd ~/shitverter
   chmod 600 telegram-api.env
   docker compose -f compose.telegram-api.yml up --detach --build
   ```

4. Set the intended processing limits in `shitverter.env`. For example, 500 MiB
   inputs and outputs with one conversion at a time:

   ```dotenv
   MAX_INPUT_BYTES=524288000
   MAX_OUTPUT_BYTES=524288000
   MAX_CONCURRENT_CONVERSIONS=1
   TELEGRAM_REQUEST_TIMEOUT_SECONDS=300
   ```

   The code's defaults remain 100 MiB input and 49,000,000 bytes output. Merely
   switching APIs does not raise them. Allow disk space for the API's cached
   source, the bot's working copy, and the converted output. Two 500 MiB copies
   plus a 500 MiB result require about 1.5 GiB, plus retained API data. This is
   not an upper bound: `MAX_OUTPUT_BYTES` is checked after FFmpeg finishes, so
   the intermediate output can grow beyond that limit. Allow additional free
   disk space for transcoding. The bot cleans its own job directories and stale
   directories on startup; it does not delete API session/cache files. Monitor
   free space in both named volumes.
5. Stop the existing bot and any other polling/webhook consumer of the same
   token. Then call cloud `logOut` once, as required by
   [Telegram's migration procedure](https://core.telegram.org/bots/api#logout).
   The following example asks for the existing bot token without echoing it or
   putting it in process arguments:

   ```bash
   docker stop shitverter
   python3 - <<'PY'
   import getpass
   import json
   import urllib.request

   token = getpass.getpass("Existing bot token: ")
   try:
       request = urllib.request.Request(
           "https://api.telegram.org/bot" + token + "/logOut", data=b""
       )
       with urllib.request.urlopen(request, timeout=30) as response:
           result = json.load(response)
       if result.get("ok") is not True or result.get("result") is not True:
           raise RuntimeError("unsuccessful logOut")
   except Exception:
       raise SystemExit("Cloud logOut failed; do not enable local mode yet.")
   print("Cloud logOut succeeded.")
   PY
   ```

6. After successful `logOut`, enable local mode and redeploy the same new image:

   ```bash
   touch local-api.enabled
   ./deploy.sh 'ghcr.io/qqrm/shitverter@sha256:REPLACE_WITH_NEW_IMAGE_DIGEST'
   ```

   `compose.local-api.yml` supplies `TELOXIDE_API_URL`,
   `TELEGRAM_LOCAL_FILES_DIR`, and `TMPDIR`; these are not manually copied into
   the secrets file. Send a test video above 20 MB and within the chosen limits.
   Check that conversion and return upload both succeed. The 15-second process
   check in `deploy.sh` only verifies startup, not a complete media round trip.

### Updates and rollback in local mode

Keep `local-api.enabled` on the VM for subsequent deployments. `deploy.sh`
applies the same local configuration when rolling back a bot image; it never
silently reconnects to the cloud. Only roll back to a version that supports
local paths. The API server and its persistent volumes are a separate lifecycle;
do not run `docker compose down --volumes` as part of a bot update.

Switching back to the cloud is a separate migration: stop the local bot, call
`logOut` on the local API endpoint to deregister that session, then remove
`local-api.enabled` and deploy in cloud mode. Telegram refuses cloud login for
10 minutes after the original cloud `logOut`. Follow the
[official server migration instructions](https://github.com/tdlib/telegram-bot-api#moving-a-bot-from-one-local-server-to-another)
and finish the session handoff before restarting the cloud poller. The `close`
and directory-copy procedure is for transferring state between two local
servers; it cannot transfer files into Telegram's cloud service. Large files
will again be subject to the cloud 20 MB download limit.
