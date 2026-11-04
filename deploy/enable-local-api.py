#!/usr/bin/env python3
"""Activate the local Bot API on the existing Docker VM, without shell secrets."""

import argparse
import ipaddress
import json
import os
from pathlib import Path
import re
import socket
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


BOT_CONTAINER = "shitverter"
API_CONTAINER = "shitverter-telegram-api"
LOCAL_ENDPOINT = "http://telegram-bot-api:8081"
CLOUD_ENDPOINT = "https://api.telegram.org"
MIN_FREE_BYTES = 10 * 1024**3
IMAGE_PATTERN = re.compile(r"ghcr\.io/(qqrm|qqrm-labs)/shitverter@sha256:[0-9a-f]{64}\Z")
TARGET_ENV = {
    "MAX_INPUT_BYTES": "2147483648",
    "MAX_OUTPUT_BYTES": "2000000000",
    "TELEGRAM_REQUEST_TIMEOUT_SECONDS": "1800",
    "MAX_CONCURRENT_CONVERSIONS": "1",
}
CHECKPOINT_STAGES = {"prepared", "logout-attempted", "cloud-logged-out", "local-authenticated", "complete"}


class ActivationError(Exception):
    """An operator-facing error whose text contains no command output or secrets."""


def run_command(args, *, env=None, timeout=180):
    command_env = os.environ.copy()
    if env:
        command_env.update(env)
    try:
        result = subprocess.run(
            args, env=command_env, check=True, capture_output=True,
            text=True, encoding="utf-8", errors="replace", timeout=timeout,
        )
    except (OSError, subprocess.SubprocessError):
        # Docker output, exception strings and deployment logs may contain secrets.
        raise ActivationError("Docker or deployment command failed; output withheld.") from None
    return result.stdout


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        return None


def bot_api(base_url, token, method):
    # Keep the VM's normal HTTPS proxy settings for Telegram's cloud endpoint.
    # Private HTTP requests must never carry the bot token through that proxy.
    handlers = [NoRedirect()]
    if base_url != CLOUD_ENDPOINT:
        try:
            parsed = urllib.parse.urlsplit(base_url)
            address = ipaddress.ip_address(parsed.hostname or "")
            if (parsed.scheme != "http" or parsed.port != 8081 or not address.is_private
                    or address.is_loopback or address.is_unspecified
                    or parsed.username or parsed.password or parsed.path or parsed.query or parsed.fragment):
                raise ValueError("invalid endpoint")
        except ValueError:
            raise ActivationError("Refusing an unexpected local Bot API endpoint.") from None
        handlers.insert(0, urllib.request.ProxyHandler({}))
    opener = urllib.request.build_opener(*handlers)
    request = urllib.request.Request(base_url + "/bot" + token + "/" + method, data=b"")
    try:
        with opener.open(request, timeout=45) as response:
            body = response.read(1024 * 1024 + 1)
        if len(body) > 1024 * 1024:
            raise ValueError("oversized response")
        result = json.loads(body)
        if not isinstance(result, dict):
            raise ValueError("invalid response")
    except (OSError, ValueError, urllib.error.URLError):
        raise ActivationError("Telegram request failed; its URL and response are withheld.") from None
    return result


def atomic_write(path, contents):
    """Replace a private file durably; fsync the directory before continuing."""
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".local-api-", delete=False) as stream:
            temporary = Path(stream.name)
            os.fchmod(stream.fileno(), 0o600)
            stream.write(contents)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        temporary = None
        directory_fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def read_credentials(path):
    try:
        file_stat = path.lstat()
        if not stat.S_ISREG(file_stat.st_mode) or stat.S_IMODE(file_stat.st_mode) != 0o600:
            raise ActivationError("The credentials file must be a regular file with mode 600.")
        if file_stat.st_uid != os.geteuid():
            raise ActivationError("The credentials file must belong to the deployment account.")
        aliases = {"API_ID": "TELEGRAM_API_ID", "API_HASH": "TELEGRAM_API_HASH",
                   "TELEGRAM_API_ID": "TELEGRAM_API_ID", "TELEGRAM_API_HASH": "TELEGRAM_API_HASH"}
        values = {}
        for line in path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            key, separator, value = line.partition("=")
            key, value = key.strip(), value.strip()
            if not separator or key not in aliases or aliases[key] in values:
                raise ActivationError("The credentials file must contain one API ID and one API hash.")
            if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
                value = value[1:-1]
            values[aliases[key]] = value
        api_id = values.get("TELEGRAM_API_ID", "")
        api_hash = values.get("TELEGRAM_API_HASH", "")
        if not re.fullmatch(r"[1-9][0-9]{0,9}", api_id) or int(api_id) > 2147483647:
            raise ActivationError("The Telegram API ID must be a positive 32-bit integer.")
        if not re.fullmatch(r"[0-9a-fA-F]{32}", api_hash):
            raise ActivationError("The Telegram API hash must contain exactly 32 hexadecimal characters.")
        return f"TELEGRAM_API_ID={api_id}\nTELEGRAM_API_HASH={api_hash.lower()}\n".encode()
    except (OSError, UnicodeError):
        raise ActivationError("Cannot read the private Telegram API credentials file.") from None


def updated_env(contents):
    # Preserve unrelated dotenv entries and comments; never execute or source them.
    try:
        lines = contents.decode("utf-8").splitlines(keepends=True)
    except UnicodeError:
        raise ActivationError("shitverter.env must be UTF-8 text.") from None
    output, written = [], set()
    for line in lines:
        assignment = re.match(r"\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=", line)
        key = assignment.group(1) if assignment else None
        if key in TARGET_ENV:
            if key not in written:
                output.append(f"{key}={TARGET_ENV[key]}\n")
                written.add(key)
        else:
            output.append(line)
    if output and not output[-1].endswith(("\n", "\r")):
        output[-1] += "\n"
    output.extend(f"{key}={value}\n" for key, value in TARGET_ENV.items() if key not in written)
    return "".join(output).encode("utf-8")


def inspect_container(name):
    try:
        value = json.loads(run_command(["docker", "inspect", name], timeout=15))
        if not isinstance(value, list) or len(value) != 1 or not isinstance(value[0], dict):
            raise ValueError("invalid inspect output")
        return value[0]
    except (ValueError, TypeError):
        raise ActivationError("Cannot inspect the required Docker container.") from None


def container_env(container):
    try:
        return dict(item.split("=", 1) for item in container["Config"]["Env"] if "=" in item)
    except (KeyError, TypeError, ValueError):
        raise ActivationError("Cannot read the current bot environment.") from None


def bot_token(environment):
    token = environment.get("TELOXIDE_TOKEN", "")
    if not re.fullmatch(r"[0-9]+:[A-Za-z0-9_-]+", token):
        raise ActivationError("The existing bot container has no valid TELOXIDE_TOKEN.")
    return token


def wait_for_api():
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        try:
            container = inspect_container(API_CONTAINER)
            address = container["NetworkSettings"]["Networks"]["shitverter-telegram"]["IPAddress"]
            ip = ipaddress.ip_address(address)
            if not ip.is_private or ip.is_loopback or ip.is_unspecified:
                raise ActivationError("The Bot API container has no private Docker network address.")
            if container["State"]["Running"]:
                with socket.create_connection((address, 8081), timeout=2):
                    return f"http://[{address}]:8081" if ip.version == 6 else f"http://{address}:8081"
        except (ActivationError, OSError, KeyError, TypeError, ValueError):
            pass
        time.sleep(2)
    raise ActivationError("The local Bot API did not become ready on its private TCP port.")


def check_disk_space():
    for volume_path in ("/var/lib/telegram-bot-api", "/var/lib/shitverter-work"):
        output = run_command(["docker", "exec", API_CONTAINER, "df", "-Pk", volume_path], timeout=15)
        try:
            available = int(output.strip().splitlines()[-1].split()[3]) * 1024
        except (IndexError, ValueError):
            raise ActivationError("Cannot determine free space in both media volumes.") from None
        if available < MIN_FREE_BYTES:
            raise ActivationError("At least 10 GiB must be free in each media volume before activation.")


def read_checkpoint(path):
    if not path.exists():
        return None
    try:
        if path.is_symlink() or not path.is_file():
            raise ValueError("invalid checkpoint file")
        value = json.loads(path.read_text(encoding="utf-8"))
        if (value.get("version") != 1 or value.get("stage") not in CHECKPOINT_STAGES
                or type(value.get("bot_id")) is not int or value["bot_id"] <= 0
                or not IMAGE_PATTERN.fullmatch(value.get("bot_image", ""))):
            raise ValueError("unknown checkpoint")
        return value
    except (OSError, ValueError, TypeError, AttributeError, UnicodeError):
        raise ActivationError("The local migration checkpoint is invalid; preserve it and investigate on the VM.") from None


def write_checkpoint(path, stage, bot_id, bot_image):
    value = {"version": 1, "stage": stage, "bot_id": bot_id, "bot_image": bot_image}
    atomic_write(path, (json.dumps(value) + "\n").encode())


def remove_checkpoint(path):
    path.unlink(missing_ok=True)
    directory_fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory_fd)
    finally:
        os.close(directory_fd)


def enable_local_api(deployment_dir, bot_image, api_image, credentials):
    deployment_dir, credentials = Path(deployment_dir).resolve(), Path(credentials)
    if not IMAGE_PATTERN.fullmatch(bot_image) or not IMAGE_PATTERN.fullmatch(api_image):
        raise ActivationError("Both images must be immutable SHA-256 digests in the approved Shitverter GHCR repository.")
    for name in ("compose.yml", "compose.local-api.yml", "compose.telegram-api.yml", "deploy.sh", "shitverter.env"):
        path = deployment_dir / name
        if not path.is_file() or path.is_symlink():
            raise ActivationError("Required deployment files are missing or are symlinks; deploy the current files first.")
    credential_contents = read_credentials(credentials)
    env_path = deployment_dir / "shitverter.env"
    backup = deployment_dir / "shitverter.env.before-local-api"
    checkpoint_path = deployment_dir / "local-api.migration.json"
    marker_path = deployment_dir / "local-api.enabled"
    checkpoint = read_checkpoint(checkpoint_path)
    original_env = env_path.read_bytes()
    candidate_env = updated_env(original_env)
    current = inspect_container(BOT_CONTAINER)
    environment = container_env(current)
    token = bot_token(environment)
    bot_id = int(token.partition(":")[0])
    if checkpoint is not None and checkpoint["bot_id"] != bot_id:
        raise ActivationError("The migration checkpoint belongs to a different bot; preserve it and check the VM identity.")
    endpoint = environment.get("TELOXIDE_API_URL", CLOUD_ENDPOINT).rstrip("/")
    already_local = endpoint == LOCAL_ENDPOINT
    if endpoint not in (CLOUD_ENDPOINT, LOCAL_ENDPOINT):
        raise ActivationError("The current bot uses a different API endpoint; migrate that server separately.")
    if marker_path.exists() and not already_local and checkpoint is None:
        raise ActivationError("The local marker disagrees with the current bot; check the VM configuration first.")
    prepared = checkpoint is not None and checkpoint["stage"] == "prepared"
    needs_baseline = checkpoint is None and not already_local
    needs_logout = not already_local and (checkpoint is None or checkpoint["stage"] in {"prepared", "logout-attempted"})
    if needs_baseline and not current.get("State", {}).get("Running"):
        raise ActivationError("Start the existing bot before first activation; a stopped bot is allowed only when resuming local mode.")
    baseline_image = checkpoint["bot_image"] if checkpoint is not None else bot_image
    if prepared:
        if current.get("Config", {}).get("Image") != baseline_image:
            raise ActivationError("The prepared bot image changed; its compatible cloud baseline must be checked on the VM.")
        if not backup.is_file() or backup.is_symlink():
            raise ActivationError("The prepared migration has no regular environment backup; preserve the checkpoint.")
        original_env = backup.read_bytes()
    if run_command(["docker", "ps", "--filter", "name=^/my_shitverter_container$",
                    "--filter", "status=running", "--format", "{{.Names}}"], timeout=15).strip():
        raise ActivationError("Stop the legacy my_shitverter_container poller before activation.")

    api_credentials_path = deployment_dir / "telegram-api.env"
    if api_credentials_path.exists() and read_credentials(api_credentials_path) != credential_contents:
        raise ActivationError("Existing Telegram API credentials differ; credential rotation requires a separate operation.")
    api_compose = ["docker", "compose", "--project-name", "shitverter-telegram",
                   "--env-file", str(deployment_dir / "telegram-api-image.env"),
                   "-f", str(deployment_dir / "compose.telegram-api.yml")]
    bot_compose = ["docker", "compose", "--project-name", "shitverter",
                   "-f", str(deployment_dir / "compose.yml"),
                   "-f", str(deployment_dir / "compose.local-api.yml")]
    bot_image_env = {"SHITVERTER_IMAGE": bot_image}
    api_image_env = {"TELEGRAM_BOT_API_IMAGE": api_image}
    handoff_started = already_local or (checkpoint is not None and not prepared)
    stopped_bot = prepared and not current.get("State", {}).get("Running")
    env_changed = prepared
    try:
        print("Preparing immutable images and the private Bot API server.", flush=True)
        run_command(["docker", "pull", bot_image])
        run_command(["docker", "pull", api_image])
        atomic_write(api_credentials_path, credential_contents)
        atomic_write(deployment_dir / "telegram-api-image.env", f"TELEGRAM_BOT_API_IMAGE={api_image}\n".encode())
        run_command(api_compose + ["config", "--quiet"], env=api_image_env)
        run_command(bot_compose + ["config", "--quiet"], env=bot_image_env)
        run_command(api_compose + ["up", "--detach", "--no-build"], env=api_image_env)
        api_url = wait_for_api()  # TCP only: no bot login before cloud logOut.
        if inspect_container(API_CONTAINER).get("Config", {}).get("Image") != api_image:
            raise ActivationError("The running Bot API image does not match the approved digest.")
        check_disk_space()

        if needs_logout and (needs_baseline or prepared):
            webhook = bot_api(CLOUD_ENDPOINT, token, "getWebhookInfo")
            if (webhook.get("ok") is not True or not isinstance(webhook.get("result"), dict)
                    or not isinstance(webhook["result"].get("url"), str)):
                raise ActivationError("Cannot verify that the bot has no active webhook.")
            if webhook["result"]["url"]:
                raise ActivationError("The bot has an active webhook. Remove or migrate that consumer before activation.")
        if needs_baseline:
            print("Verifying the selected bot image in cloud mode before migration.", flush=True)
            run_command([str(deployment_dir / "deploy.sh"), bot_image])
            baseline = inspect_container(BOT_CONTAINER)
            environment = container_env(baseline)
            if (not baseline.get("State", {}).get("Running")
                    or baseline.get("Config", {}).get("Image") != bot_image
                    or environment.get("TELOXIDE_API_URL", CLOUD_ENDPOINT).rstrip("/") != CLOUD_ENDPOINT):
                raise ActivationError("The compatible cloud-mode baseline could not be verified.")
            if bot_token(environment) != token:
                raise ActivationError("The selected cloud baseline has a different bot token; reconcile the VM environment first.")
            baseline_image = bot_image

        if needs_baseline or not backup.exists():
            atomic_write(backup, original_env)
        elif backup.is_symlink() or not backup.is_file():
            raise ActivationError("The environment backup is not a regular private file.")
        else:
            backup.chmod(0o600)
        if needs_baseline:
            # A killed SSH session can safely resume even after docker stop.
            # This stage explicitly means no cloud logOut has been attempted.
            write_checkpoint(checkpoint_path, "prepared", bot_id, baseline_image)
        env_changed = True
        atomic_write(env_path, candidate_env)
        run_command(bot_compose + ["config", "--quiet"], env=bot_image_env)
        print("Stopping the existing poller for the local API handoff.", flush=True)
        stopped_bot = True  # Also attempt recovery if Docker stops it but loses the reply.
        run_command(["docker", "stop", BOT_CONTAINER], timeout=60)

        if needs_logout:
            # Mark the ambiguous window durably BEFORE transmitting any logOut request.
            write_checkpoint(checkpoint_path, "logout-attempted", bot_id, baseline_image)
            handoff_started = True
            result = bot_api(CLOUD_ENDPOINT, token, "logOut")
            if result.get("ok") is not True or result.get("result") is not True:
                raise ActivationError("Cloud logOut did not return success; its response is withheld.")
            write_checkpoint(checkpoint_path, "cloud-logged-out", bot_id, baseline_image)
        result = bot_api(api_url, token, "getMe")
        if result.get("ok") is not True or not isinstance(result.get("result"), dict):
            raise ActivationError("The local Bot API could not authenticate the bot.")
        write_checkpoint(checkpoint_path, "local-authenticated", bot_id, baseline_image)
        handoff_started = True
        atomic_write(marker_path, b"local Bot API enabled\n")
        print("Deploying the bot with the local endpoint and large-file limits.", flush=True)
        run_command([str(deployment_dir / "deploy.sh"), bot_image])
        deployed = inspect_container(BOT_CONTAINER)
        deployed_env = container_env(deployed)
        expected = {**TARGET_ENV, "TELOXIDE_API_URL": LOCAL_ENDPOINT,
                    "TELEGRAM_LOCAL_FILES_DIR": "/var/lib/telegram-bot-api",
                    "TMPDIR": "/var/lib/shitverter-work", "TELOXIDE_TOKEN": token}
        if (not deployed.get("State", {}).get("Running")
                or deployed.get("Config", {}).get("Image") != bot_image
                or any(deployed_env.get(key) != value for key, value in expected.items())):
            raise ActivationError("The deployed bot's image, endpoint or large-file limits could not be verified.")
        write_checkpoint(checkpoint_path, "complete", bot_id, bot_image)
        print("Local Bot API activation succeeded. Send a video above 20 MB to verify the full conversion.", flush=True)
    except BaseException as error:
        # A replacement may have succeeded even when the following fsync failed.
        # An unreadable checkpoint is also insufficient evidence for cloud rollback.
        try:
            recorded = read_checkpoint(checkpoint_path)
            handoff_started = handoff_started or (recorded is not None and recorded["stage"] != "prepared")
        except ActivationError:
            handoff_started = True
        if not handoff_started:
            recovery_failed = False
            try:
                if env_changed:
                    atomic_write(env_path, original_env)
                if stopped_bot:
                    run_command(["docker", "start", BOT_CONTAINER], timeout=60)
                if checkpoint_path.exists():
                    remove_checkpoint(checkpoint_path)
            except (ActivationError, OSError):
                recovery_failed = True
            if recovery_failed:
                raise ActivationError("Activation failed before cloud logOut; automatic cloud-bot recovery also failed. Check the VM.") from None
        if isinstance(error, (KeyboardInterrupt, SystemExit)):
            raise
        detail = str(error) if isinstance(error, ActivationError) else "Local activation encountered an operating-system error."
        if handoff_started:
            raise ActivationError(detail + " Local handoff is recorded: keep the checkpoint and rerun this command; do not restart cloud mode. If cloud logOut is unconfirmed and Telegram refuses the retry, wait for its cooldown (at least 10 minutes after the attempt) before retrying.") from None
        raise ActivationError(detail + " No cloud logOut was attempted; the previous bot configuration was preserved.") from None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-api-ready", action="store_true", help="only check the private API TCP endpoint")
    parser.add_argument("--bot-image")
    parser.add_argument("--api-image")
    parser.add_argument("--credentials", type=Path)
    args = parser.parse_args(argv)
    if args.check_api_ready:
        if args.bot_image or args.api_image or args.credentials:
            parser.error("--check-api-ready cannot be combined with activation arguments")
    elif not (args.bot_image and args.api_image and args.credentials):
        parser.error("activation requires --bot-image, --api-image and --credentials")
    try:
        if args.check_api_ready:
            wait_for_api()
            print("The local Bot API TCP endpoint is ready.")
        else:
            enable_local_api(Path(__file__).resolve().parent, args.bot_image, args.api_image, args.credentials)
    except ActivationError as error:
        print(f"Activation failed: {error}", file=sys.stderr)
        return 1
    except (OSError, KeyboardInterrupt):
        print("Activation interrupted or files unavailable. Preserve any migration checkpoint and rerun the same command.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
