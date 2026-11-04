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

Keep the repository's `production` environment, require an approval
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
does not need sudo. Normal deployments copy the deployment scripts, Compose files,
and local API Dockerfile while preserving the VM's environment files and mode.
The explicit `enable_local_api` option performs the one-time configuration below.
That option also requires Python 3 on the VM; Ubuntu includes it by default.

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

### Enable large files from GitHub Actions

No additional SSH credentials or manual Docker commands are needed when ordinary
Deploy already works. Obtain your own application `api_id` and `api_hash` from
[my.telegram.org](https://core.telegram.org/api/obtaining_api_id), then:

1. Add `TELEGRAM_API_ID` and `TELEGRAM_API_HASH` as secrets in the `production`
   environment. The existing bot token stays on the VM in `shitverter.env`.
2. Run **Deploy** from `main`: leave `publish_only` unchecked and check
   **Enable files up to 2 GiB** (`enable_local_api`). Approve `production` when
   GitHub asks, as for an ordinary deployment.
3. After success, send a video above 20 MB and then a representative large video.
   A successful startup does not establish that an actual 2 GiB conversion has
   completed; test download, FFmpeg conversion, and return upload together.

The workflow validates the new secrets before touching the VM. It builds the
official API image on a GitHub runner, publishes it under
`ghcr.io/OWNER/shitverter:telegram-api-sha-COMMIT`, and deploys by immutable digest.
Using the existing GHCR package preserves the VM's existing pull access; the VM
does not compile C++. The first API build may take around 25 minutes. The bot
keeps running during that build.

The activation helper writes `telegram-api.env` and `telegram-api-image.env` with
mode 600. Before stopping the cloud poller it pulls and validates both images,
starts the isolated API container, checks its listener without logging the bot
into it, and requires at least 10 GiB of available space on both mounted volumes.
It first installs the selected bot image in cloud mode to establish a compatible
rollback version, then switches APIs using Telegram's required cloud `logOut`.
The helper checks local `getMe` before starting the bot in local mode and verifies
the resulting container's configuration.

Activation backs up `shitverter.env` and sets these values:

| Setting | Value |
| --- | --- |
| `MAX_INPUT_BYTES` | `2147483648` (2 GiB, inclusive) |
| `MAX_OUTPUT_BYTES` | `2000000000` (within Telegram's documented 2000 MB upload limit) |
| `TELEGRAM_REQUEST_TIMEOUT_SECONDS` | `1800` (30 minutes) |
| `MAX_CONCURRENT_CONVERSIONS` | `1` |

These are deployment settings; the application's cloud-mode defaults stay
unchanged. Ten GiB is a preflight floor, not a bound on disk usage: the API cache,
the bot's input copy, and FFmpeg output coexist, and output size is checked only
after conversion. Monitor free space and retained API data.

The helper records `local-api.migration.json`, bound to the bot's identity, before
stopping it and again before attempting cloud `logOut`.
If the response is lost or a later step fails, rerun **Deploy** with
`enable_local_api` checked. It resumes local activation instead of restarting a
cloud poller that Telegram may already have logged out. An unconfirmed logout
must receive a positive acknowledgement on retry; if Telegram applies its
10-minute cloud-login cooldown, wait for that window and retry. The helper never
treats a network failure or rate limit as proof of a successful logout. An ordinary deployment
refuses to start cloud mode while this checkpoint exists without a local marker.
After activation, leave the option unchecked for routine bot deployments; they
preserve local mode and do not rebuild or replace a running API container. If the
API container is stopped, `deploy.sh` restores it from its saved digest.

### Manual setup and migration

The steps below are an alternative for operators using the VM console directly.

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

4. Set the intended processing limits in `shitverter.env`. For example, inputs
   up to 2 GiB and outputs up to 2000 MB with one conversion at a time:

   ```dotenv
   MAX_INPUT_BYTES=2147483648
   MAX_OUTPUT_BYTES=2000000000
   MAX_CONCURRENT_CONVERSIONS=1
   TELEGRAM_REQUEST_TIMEOUT_SECONDS=1800
   ```

   The code's defaults remain 100 MiB input and 49,000,000 bytes output. Merely
   switching APIs does not raise them. Allow disk space for the API's cached
   source, the bot's working copy, and the converted output. Two 2 GiB copies
   plus a 2000 MB result require almost 6 GiB, plus retained API data. This is
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
`local-api.enabled` and archive/remove `local-api.migration.json` if present
before deploying in cloud mode. Removing these files without completing the
session handoff is not a recovery procedure. Telegram refuses cloud login for
10 minutes after the original cloud `logOut`. Follow the
[official server migration instructions](https://github.com/tdlib/telegram-bot-api#moving-a-bot-from-one-local-server-to-another)
and finish the session handoff before restarting the cloud poller. The `close`
and directory-copy procedure is for transferring state between two local
servers; it cannot transfer files into Telegram's cloud service. Large files
will again be subject to the cloud 20 MB download limit.
