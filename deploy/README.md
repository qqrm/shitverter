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

Use `chmod 600 ~/shitverter/shitverter.env`. Before the first workflow run,
the VM must be allowed to pull `ghcr.io/qqrm/shitverter`. If the package is
private, log in once with a fine-grained token limited to `Packages: Read` for
this repository:

```bash
printf '%s' "$GHCR_READ_TOKEN" | docker login ghcr.io --username qqrm --password-stdin
```

Create a dedicated SSH key for deployment. Add its public half to the
deployment account's `~/.ssh/authorized_keys`; do not reuse a personal key.

## GitHub environment

Create a `production` environment in `qqrm/shitverter`, require an approval
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
does not need sudo. The workflow copies only `compose.yml` and `deploy.sh`; it
never writes `shitverter.env`.

## First deployment and rollback

Run **Deploy** from the `main` commit. The workflow builds and pushes a
linux/amd64 image, then passes its SHA-256 image digest to the VM. The remote
script refuses any other registry/name format, verifies that the container is
still running after five seconds, and restores the previous immutable image if
the replacement exits.

After a successful first deployment, confirm the bot accepts a small video and
then decide whether to add `push: branches: [main]` to
`.github/workflows/deploy.yml`. Keeping manual promotion is safer for a bot
without an HTTP health endpoint.
