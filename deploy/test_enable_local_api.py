"""Isolated migration/recovery checks; never contact Docker or Telegram."""

import contextlib
import importlib.util
import io
import json
import multiprocessing
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import traceback
import unittest
from unittest import mock


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "enable_local_api", HERE / "enable-local-api.py"
)
activation = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = activation
SPEC.loader.exec_module(activation)

BOT_IMAGE = "ghcr.io/qqrm/shitverter@sha256:" + "a" * 64
API_IMAGE = "ghcr.io/qqrm/shitverter@sha256:" + "b" * 64
OLD_IMAGE = "ghcr.io/qqrm/shitverter@sha256:" + "c" * 64
BOT_ID = 123456789
TOKEN = str(BOT_ID) + ":" + "T" * 35
API_HASH = "0123456789abcdef0123456789abcdef"
CREDENTIALS = f"TELEGRAM_API_ID=123456\nTELEGRAM_API_HASH={API_HASH}\n"


class DeploymentGuardTests(unittest.TestCase):
    def test_incomplete_migration_blocks_regular_deploy_before_any_docker_command(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            script = root / "deploy.sh"
            shutil.copyfile(HERE / "deploy.sh", script)
            (root / "compose.yml").write_text("services: {}\n", encoding="utf-8")
            (root / "shitverter.env").write_text("TELOXIDE_TOKEN=unused\n", encoding="utf-8")
            (root / "local-api.migration.json").write_text(
                json.dumps({
                    "version": 1, "stage": "logout-attempted",
                    "bot_id": BOT_ID, "bot_image": BOT_IMAGE,
                }),
                encoding="utf-8",
            )
            executable_dir = root / "bin"
            executable_dir.mkdir()
            docker = executable_dir / "docker"
            docker.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$*" >> "$DOCKER_CALL_LOG"\nexit 97\n',
                encoding="utf-8",
            )
            docker.chmod(0o700)
            command_log = root / "docker-called"
            result = subprocess.run(
                ["bash", str(script), BOT_IMAGE],
                env={
                    "PATH": str(executable_dir) + os.pathsep + os.defpath,
                    "DOCKER_CALL_LOG": str(command_log),
                },
                capture_output=True,
                text=True,
                timeout=10,
                check=False,
            )
            self.assertEqual(result.returncode, 69, result.stderr)
            self.assertFalse(command_log.exists(), "Docker ran before the recovery guard")


class CommandLineTests(unittest.TestCase):
    def test_readiness_only_needs_no_credentials_and_cannot_activate_the_bot(self):
        with (
            mock.patch.object(activation, "wait_for_api", return_value="http://172.18.0.2:8081") as ready,
            mock.patch.object(activation, "enable_local_api") as enable,
            mock.patch.object(activation, "run_command", side_effect=AssertionError("Unexpected command")),
            contextlib.redirect_stdout(io.StringIO()),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            self.assertEqual(activation.main(["--check-api-ready"]), 0)
            ready.assert_called_once_with()
            enable.assert_not_called()
            with self.assertRaises(SystemExit) as caught:
                activation.main(["--check-api-ready", "--bot-image", BOT_IMAGE])
            self.assertEqual(caught.exception.code, 2)
            ready.assert_called_once_with()
            enable.assert_not_called()


class TransportRedactionTests(unittest.TestCase):
    def assert_redacted(self, error):
        visible = "".join(traceback.format_exception(error))
        self.assertNotIn(TOKEN, visible)
        self.assertNotIn(API_HASH, visible)

    def test_command_failures_and_timeouts_hide_raw_output_and_exception_context(self):
        command = ["docker", "compose", "up"]
        failures = [
            subprocess.CalledProcessError(
                1, command, output=f"TELOXIDE_TOKEN={TOKEN}", stderr=f"API_HASH={API_HASH}"
            ),
            subprocess.TimeoutExpired(
                command, 180, output=f"TELOXIDE_TOKEN={TOKEN}", stderr=f"API_HASH={API_HASH}"
            ),
            OSError(f"Cannot run command for {TOKEN}, {API_HASH}"),
        ]
        for failure in failures:
            with self.subTest(kind=type(failure).__name__):
                with mock.patch.object(activation.subprocess, "run", side_effect=failure):
                    with self.assertRaises(activation.ActivationError) as caught:
                        activation.run_command(command)
                self.assert_redacted(caught.exception)

    def test_http_and_invalid_json_errors_hide_token_url_and_response_body(self):
        for base_url in ("https://api.telegram.org", "http://172.18.0.2:8081"):
            with self.subTest(endpoint=base_url):
                error = activation.urllib.error.HTTPError(
                    base_url + "/bot" + TOKEN + "/getMe",
                    400,
                    "Rejected " + API_HASH,
                    {},
                    io.BytesIO(f"TELOXIDE_TOKEN={TOKEN}".encode()),
                )
                with mock.patch("urllib.request.OpenerDirector.open", side_effect=error):
                    with self.assertRaises(activation.ActivationError) as caught:
                        activation.bot_api(base_url, TOKEN, "getMe")
                self.assert_redacted(caught.exception)

                response = mock.MagicMock()
                response.__enter__.return_value.read.return_value = (
                    "invalid-json " + TOKEN + " " + API_HASH
                ).encode()
                with mock.patch("urllib.request.OpenerDirector.open", return_value=response):
                    with self.assertRaises(activation.ActivationError) as caught:
                        activation.bot_api(base_url, TOKEN, "getMe")
                self.assert_redacted(caught.exception)


class MigrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.deployment = self.root / "deploy"
        self.deployment.mkdir()
        for name in (
            "compose.yml",
            "compose.local-api.yml",
            "compose.telegram-api.yml",
            "deploy.sh",
        ):
            shutil.copyfile(HERE / name, self.deployment / name)
        self.original_env = (
            "# Preserve the operator's existing settings.\n"
            f"TELOXIDE_TOKEN={TOKEN}\n"
            "USER_DAILY_LIMIT=17\n"
            "CUSTOM_SETTING='value with spaces = retained'\n"
            "MAX_INPUT_BYTES=104857600\n"
            "MAX_OUTPUT_BYTES=49000000\n"
            "TELEGRAM_REQUEST_TIMEOUT_SECONDS=300\n"
        )
        self.env_path = self.deployment / "shitverter.env"
        self.env_path.write_text(self.original_env, encoding="utf-8")
        self.env_path.chmod(0o600)
        self.credentials = self.root / "credentials.env"
        self.credentials.write_text(CREDENTIALS, encoding="utf-8")
        self.credentials.chmod(0o600)
        self.events = []
        self.deploy_observations = []
        self.fail_command = None
        self.fail_api = None
        self.bot_running = True
        self.legacy_running = False
        self.current_image = OLD_IMAGE
        self.current_environment = {"TELOXIDE_TOKEN": TOKEN}
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.stdout = io.StringIO()
        self.stderr = io.StringIO()
        self.stack.enter_context(contextlib.redirect_stdout(self.stdout))
        self.stack.enter_context(contextlib.redirect_stderr(self.stderr))
        self.stack.enter_context(mock.patch.dict(os.environ, {}, clear=True))
        self.stack.enter_context(
            mock.patch("socket.create_connection", side_effect=AssertionError("Unexpected network"))
        )
        self.stack.enter_context(
            mock.patch("urllib.request.OpenerDirector.open", side_effect=AssertionError("Unexpected HTTP"))
        )
        self.stack.enter_context(
            mock.patch.object(
                activation.subprocess,
                "run",
                side_effect=AssertionError("Unexpected real subprocess"),
            )
        )
        self.stack.enter_context(
            mock.patch.object(activation, "run_command", side_effect=self.command)
        )
        self.stack.enter_context(
            mock.patch.object(activation, "bot_api", side_effect=self.api)
        )
        self.stack.enter_context(
            mock.patch.object(activation, "wait_for_api", side_effect=self.ready)
        )
        self.stack.enter_context(
            mock.patch.object(activation, "check_disk_space", side_effect=self.disk)
        )

    def enable(self):
        activation.enable_local_api(
            self.deployment, BOT_IMAGE, API_IMAGE, self.credentials
        )

    def command(self, args, *, env=None, timeout=180):
        args = [str(value) for value in args]
        self.events.append(("command", args))
        if self.fail_command is not None:
            self.fail_command(args)
        if args[:2] == ["docker", "inspect"]:
            if len(args) == 3:
                is_api = args[-1] == "shitverter-telegram-api"
                return json.dumps([{
                    "Config": {
                        "Image": API_IMAGE if is_api else self.current_image,
                        "Env": [f"{key}={value}" for key, value in self.current_environment.items()],
                    },
                    "State": {"Running": True if is_api else self.bot_running, "Restarting": False},
                    "RestartCount": 0,
                }])
            if ".Config.Image" in " ".join(args):
                return self.current_image
            if ".State.Running" in " ".join(args):
                running = (
                    self.legacy_running
                    if args[-1] == "my_shitverter_container"
                    else self.bot_running
                )
                return str(running).lower()
            self.fail(f"Unhandled Docker inspect fixture: {args!r}")
        if args[:2] == ["docker", "ps"]:
            return "my_shitverter_container\n" if self.legacy_running else ""
        if args[:2] == ["docker", "stop"]:
            if "shitverter" in args[2:]:
                self.bot_running = False
            if "my_shitverter_container" in args[2:]:
                self.legacy_running = False
            return ""
        if args[:2] == ["docker", "start"]:
            if "shitverter" in args[2:]:
                self.bot_running = True
            if "my_shitverter_container" in args[2:]:
                self.legacy_running = True
            return ""
        if any(Path(value).name == "deploy.sh" for value in args):
            self.deploy_observations.append({
                "local": (self.deployment / "local-api.enabled").exists(),
                "env": self.env_path.read_text(encoding="utf-8"),
            })
            self.bot_running = True
            self.current_image = BOT_IMAGE
            self.current_environment = dict(
                line.split("=", 1)
                for line in self.env_path.read_text(encoding="utf-8").splitlines()
                if "=" in line and not line.startswith("#")
            )
            if (self.deployment / "local-api.enabled").exists():
                self.current_environment.update({
                    "TELOXIDE_API_URL": "http://telegram-bot-api:8081",
                    "TELEGRAM_LOCAL_FILES_DIR": "/var/lib/telegram-bot-api",
                    "TMPDIR": "/var/lib/shitverter-work",
                })
            return ""
        if args[:2] in (["docker", "compose"], ["docker", "pull"]):
            return ""
        self.fail(f"Unhandled command fixture: {args!r}")

    def api(self, base_url, token, method):
        self.assertEqual(token, TOKEN)
        self.events.append(("api", base_url, method))
        if self.fail_api is not None:
            response = self.fail_api(base_url, method)
            if response is not None:
                return response
        result = {"id": BOT_ID}
        if method == "logOut":
            result = True
        elif method == "getWebhookInfo":
            result = {"url": ""}
        return {"ok": True, "result": result}

    def ready(self):
        self.events.append(("api-ready",))
        return "http://172.18.0.2:8081"

    def disk(self):
        self.events.append(("disk-ready",))

    def logout_events(self):
        return [event for event in self.events if event[0] == "api" and event[2] == "logOut"]

    def checkpoint(self):
        return json.loads(
            (self.deployment / "local-api.migration.json").read_text(encoding="utf-8")
        )

    def seed_checkpoint(self, stage, *, bot_id=BOT_ID, bot_image=BOT_IMAGE):
        (self.deployment / "local-api.migration.json").write_text(
            json.dumps({
                "version": 1, "stage": stage, "bot_id": bot_id, "bot_image": bot_image,
            }),
            encoding="utf-8",
        )

    def local_auth_events(self):
        return [
            event for event in self.events
            if event[0] == "api"
            and event[1] != "https://api.telegram.org"
            and event[2] == "getMe"
        ]

    def assert_process_killed(self, action):
        # Fork preserves the isolated Docker/HTTP fakes. SIGKILL bypasses all
        # Python exception handlers, exposing the durable state used on retry.
        process = multiprocessing.get_context("fork").Process(target=action)
        process.start()
        process.join(timeout=10)
        if process.is_alive():
            process.kill()
            process.join(timeout=10)
            process.close()
            self.fail("The crash fixture did not reach its interruption point")
        exitcode = process.exitcode
        process.close()
        self.assertEqual(exitcode, -signal.SIGKILL)

    def stopped_commands(self):
        return [
            event for event in self.events
            if event[0] == "command" and event[1][:2] == ["docker", "stop"]
        ]

    def assert_no_cloud_restart(self):
        self.assertFalse(any(
            event[0] == "command" and event[1][:2] == ["docker", "start"]
            for event in self.events
        ))
        self.assertTrue(all(
            observation["local"] for observation in self.deploy_observations
        ))

    def assert_target_limits(self):
        lines = self.env_path.read_text(encoding="utf-8").splitlines()
        expected = {
            "MAX_INPUT_BYTES": "2147483648",
            "MAX_OUTPUT_BYTES": "2000000000",
            "TELEGRAM_REQUEST_TIMEOUT_SECONDS": "1800",
        }
        for key, value in expected.items():
            with self.subTest(key=key):
                self.assertEqual(
                    [line for line in lines if line.startswith(key + "=")],
                    [f"{key}={value}"],
                )
        self.assertIn(f"TELOXIDE_TOKEN={TOKEN}", lines)
        self.assertIn("USER_DAILY_LIMIT=17", lines)
        self.assertIn("CUSTOM_SETTING='value with spaces = retained'", lines)
        self.assertIn("# Preserve the operator's existing settings.", lines)

    def test_success_prepares_local_api_before_logging_out_and_starts_local_bot(self):
        def observe_logout(base_url, method):
            if method == "logOut":
                self.assertFalse(self.bot_running)
                self.assertEqual(self.checkpoint()["stage"], "logout-attempted")
                self.assertEqual(self.checkpoint()["bot_id"], BOT_ID)
                self.assertEqual(self.checkpoint()["bot_image"], BOT_IMAGE)
                self.assertIn(("api-ready",), self.events)
                self.assertIn(("disk-ready",), self.events)
                self.assertEqual(len(self.deploy_observations), 1)
                self.assertFalse(self.deploy_observations[0]["local"])
                self.assertEqual(self.deploy_observations[0]["env"], self.original_env)
            elif method == "getMe" and base_url != "https://api.telegram.org":
                self.assertEqual(self.checkpoint()["stage"], "cloud-logged-out")

        self.fail_api = observe_logout
        write = activation.atomic_write

        def observe_env_write(path, contents):
            if path == self.env_path:
                self.assertEqual(self.checkpoint()["stage"], "prepared")
            return write(path, contents)

        with mock.patch.object(activation, "atomic_write", side_effect=observe_env_write):
            self.enable()

        self.assertEqual(len(self.logout_events()), 1)
        logout_index = self.events.index(self.logout_events()[0])
        local_auth = [
            i for i, event in enumerate(self.events)
            if event[0] == "api"
            and event[1] != "https://api.telegram.org"
            and event[2] == "getMe"
        ]
        self.assertTrue(local_auth)
        self.assertGreater(min(local_auth), logout_index)
        self.assertTrue(self.bot_running)
        self.assertTrue(self.deploy_observations[-1]["local"])
        self.assertEqual(self.checkpoint()["stage"], "complete")
        self.assert_target_limits()
        backup = self.deployment / "shitverter.env.before-local-api"
        self.assertEqual(backup.read_text(encoding="utf-8"), self.original_env)

    def test_missing_credentials_refuses_before_redeploy_stop_or_logout(self):
        self.credentials.unlink()
        with self.assertRaises(activation.ActivationError):
            self.enable()
        self.assertFalse(self.stopped_commands())
        self.assertFalse(self.deploy_observations)
        self.assertFalse(self.logout_events())
        self.assertTrue(self.bot_running)
        self.assertEqual(self.env_path.read_text(encoding="utf-8"), self.original_env)

    def test_invalid_credentials_are_rejected_without_echoing_secret_text(self):
        invalid = [
            f"TELEGRAM_API_ID=not-a-number\nTELEGRAM_API_HASH={API_HASH}\n",
            f"TELEGRAM_API_ID=0\nTELEGRAM_API_HASH={API_HASH}\n",
            "TELEGRAM_API_ID=123456\nTELEGRAM_API_HASH=not-a-hash\n",
            f"TELEGRAM_API_ID=123456\nTELEGRAM_API_HASH={API_HASH}\nUNEXPECTED={TOKEN}\n",
        ]
        for text in invalid:
            with self.subTest(case=invalid.index(text)):
                self.credentials.write_text(text, encoding="utf-8")
                with self.assertRaises(activation.ActivationError) as caught:
                    self.enable()
                self.assertNotIn(TOKEN, str(caught.exception))
                self.assertNotIn(API_HASH, str(caught.exception))
        self.assertFalse(self.stopped_commands())
        self.assertFalse(self.deploy_observations)
        self.assertFalse(self.logout_events())

    def test_credentials_accept_documented_aliases_and_quoted_values(self):
        self.credentials.write_text(
            f"# Supplied by Actions\nAPI_ID='123456'\nAPI_HASH=\"{API_HASH.upper()}\"\n",
            encoding="utf-8",
        )
        self.assertEqual(activation.read_credentials(self.credentials), CREDENTIALS.encode())

    def test_corrupt_checkpoint_is_preserved_and_refuses_before_container_changes(self):
        checkpoint = self.deployment / "local-api.migration.json"
        contents = '{"version":999,"stage":"complete"}\n'
        checkpoint.write_text(contents, encoding="utf-8")
        with self.assertRaises(activation.ActivationError):
            self.enable()
        self.assertEqual(checkpoint.read_text(encoding="utf-8"), contents)
        self.assertFalse(self.stopped_commands())
        self.assertFalse(self.deploy_observations)
        self.assertFalse(self.logout_events())

    def test_failed_preflight_leaves_cloud_bot_and_environment_untouched(self):
        with mock.patch.object(
            activation, "check_disk_space",
            side_effect=activation.ActivationError("Insufficient disk space"),
        ):
            with self.assertRaises(activation.ActivationError):
                self.enable()
        self.assertTrue(self.bot_running)
        self.assertFalse(self.stopped_commands())
        self.assertFalse(self.deploy_observations)
        self.assertFalse(self.logout_events())
        self.assertFalse((self.deployment / "local-api.migration.json").exists())
        self.assertFalse((self.deployment / "local-api.enabled").exists())
        self.assertEqual(self.env_path.read_text(encoding="utf-8"), self.original_env)

    def test_failed_compatible_cloud_baseline_never_calls_logout(self):
        def fail_baseline(args):
            if any(Path(value).name == "deploy.sh" for value in args):
                raise activation.ActivationError("Candidate baseline failed")

        self.fail_command = fail_baseline
        with self.assertRaises(activation.ActivationError):
            self.enable()
        self.assertFalse(self.logout_events())
        self.assertFalse(self.stopped_commands())
        self.assertFalse((self.deployment / "local-api.enabled").exists())
        self.assertEqual(self.env_path.read_text(encoding="utf-8"), self.original_env)

    def test_lost_stop_response_before_logout_restores_cloud_bot_and_original_env(self):
        def stop_then_lose_response(args):
            if args[:2] == ["docker", "stop"]:
                self.bot_running = False
                raise activation.ActivationError("Docker stop response lost")

        self.fail_command = stop_then_lose_response
        with self.assertRaises(activation.ActivationError):
            self.enable()
        self.assertTrue(self.bot_running)
        self.assertFalse(self.logout_events())
        self.assertFalse((self.deployment / "local-api.migration.json").exists())
        self.assertFalse((self.deployment / "local-api.enabled").exists())
        self.assertEqual(self.env_path.read_text(encoding="utf-8"), self.original_env)
        self.assertEqual(
            [event[1] for event in self.events
             if event[0] == "command" and event[1][:2] == ["docker", "start"]],
            [["docker", "start", "shitverter"]],
        )

    def test_failed_checkpoint_before_logout_restores_cloud_instead_of_sending_request(self):
        with mock.patch.object(activation, "write_checkpoint", side_effect=OSError("Disk failure")):
            with self.assertRaises(activation.ActivationError):
                self.enable()
        self.assertTrue(self.bot_running)
        self.assertFalse(self.logout_events())
        self.assertFalse((self.deployment / "local-api.enabled").exists())
        self.assertEqual(self.env_path.read_text(encoding="utf-8"), self.original_env)

    def test_persisted_logout_checkpoint_error_does_not_restart_cloud(self):
        write = activation.write_checkpoint

        def persist_then_fail(path, stage, bot_id, bot_image):
            write(path, stage, bot_id, bot_image)
            if stage == "logout-attempted":
                raise OSError("Directory fsync failed after replace")

        with mock.patch.object(activation, "write_checkpoint", side_effect=persist_then_fail):
            with self.assertRaises(activation.ActivationError):
                self.enable()
        self.assertEqual(self.checkpoint()["stage"], "logout-attempted")
        self.assertFalse(self.bot_running)
        self.assertFalse(self.logout_events())
        self.assertFalse(self.local_auth_events())
        self.assertFalse(any(
            event[0] == "command" and event[1][:2] == ["docker", "start"]
            for event in self.events
        ))

        self.events.clear()
        self.deploy_observations.clear()
        self.enable()
        self.assertEqual(len(self.logout_events()), 1)
        self.assert_no_cloud_restart()
        self.assertEqual(self.checkpoint()["stage"], "complete")

    def test_lost_logout_response_and_retry_cooldown_block_local_login_until_confirmation(self):
        def lose_response(base_url, method):
            if method == "logOut":
                self.assertEqual(self.checkpoint()["stage"], "logout-attempted")
                raise activation.ActivationError("Response lost")

        self.fail_api = lose_response
        with self.assertRaises(activation.ActivationError):
            self.enable()
        self.assertEqual(self.checkpoint()["stage"], "logout-attempted")
        self.assertFalse(self.bot_running)
        self.assertFalse(any(
            event[0] == "command" and event[1][:2] == ["docker", "start"]
            for event in self.events
        ))
        self.assertFalse(self.local_auth_events())

        first_logout_count = len(self.logout_events())
        self.events.clear()
        self.deploy_observations.clear()

        def cooldown(base_url, method):
            if method == "logOut":
                return {
                    "ok": False, "error_code": 429,
                    "description": "Retry later", "parameters": {"retry_after": 600},
                }

        self.fail_api = cooldown
        with self.assertRaises(activation.ActivationError):
            self.enable()
        self.assertEqual(len(self.logout_events()), 1)
        self.assertEqual(self.checkpoint()["stage"], "logout-attempted")
        self.assertFalse(self.bot_running)
        self.assertFalse(self.local_auth_events())
        self.assertFalse((self.deployment / "local-api.enabled").exists())
        self.assert_no_cloud_restart()

        self.events.clear()
        self.deploy_observations.clear()
        self.fail_api = None
        self.enable()
        self.assertEqual(first_logout_count, 1)
        self.assertEqual(len(self.logout_events()), 1)
        self.assertTrue(self.local_auth_events())
        self.assertLess(
            self.events.index(self.logout_events()[0]),
            self.events.index(self.local_auth_events()[0]),
        )
        self.assert_no_cloud_restart()
        self.assertTrue(self.bot_running)
        self.assertEqual(self.checkpoint()["stage"], "complete")
        self.assert_target_limits()

    def test_local_auth_failure_keeps_checkpoint_and_does_not_restore_cloud(self):
        def fail_local_auth(base_url, method):
            if method == "getMe" and base_url != "https://api.telegram.org":
                raise activation.ActivationError("Local authorization unavailable")

        self.fail_api = fail_local_auth
        with self.assertRaises(activation.ActivationError):
            self.enable()
        self.assertEqual(len(self.logout_events()), 1)
        self.assertEqual(self.checkpoint()["stage"], "cloud-logged-out")
        self.assertFalse(self.bot_running)
        self.assertFalse(any(
            event[0] == "command" and event[1][:2] == ["docker", "start"]
            for event in self.events
        ))

        self.events.clear()
        self.deploy_observations.clear()
        self.fail_api = None
        self.enable()
        self.assertFalse(self.logout_events())
        self.assert_no_cloud_restart()
        self.assertEqual(self.checkpoint()["stage"], "complete")

    def test_confirmed_checkpoint_resumes_without_marker_or_another_cloud_logout(self):
        self.bot_running = False
        self.current_image = BOT_IMAGE
        self.seed_checkpoint("cloud-logged-out")
        self.assertFalse((self.deployment / "local-api.enabled").exists())
        self.enable()
        self.assertFalse(self.logout_events())
        self.assert_no_cloud_restart()
        self.assertTrue((self.deployment / "local-api.enabled").exists())
        self.assertEqual(self.checkpoint()["stage"], "complete")
        self.assert_target_limits()

    def test_killed_before_sending_logout_retries_cloud_logout_before_local_auth(self):
        def kill_before_send(base_url, token, method):
            if method == "logOut":
                os.kill(os.getpid(), signal.SIGKILL)
            return self.api(base_url, token, method)

        with mock.patch.object(activation, "bot_api", side_effect=kill_before_send):
            self.assert_process_killed(self.enable)
        self.assertEqual(self.checkpoint()["stage"], "logout-attempted")
        self.assertEqual(self.checkpoint()["bot_id"], BOT_ID)
        self.assertFalse((self.deployment / "local-api.enabled").exists())
        # The child's fake Docker state is process-local; the persistent files
        # above are real. Reconstruct the stopped compatible bot for the retry.
        self.bot_running = False
        self.current_image = BOT_IMAGE
        self.enable()
        self.assertEqual(len(self.logout_events()), 1)
        self.assertTrue(self.local_auth_events())
        self.assertLess(
            self.events.index(self.logout_events()[0]),
            self.events.index(self.local_auth_events()[0]),
        )
        self.assert_no_cloud_restart()
        self.assertEqual(self.checkpoint()["stage"], "complete")

    def test_killed_after_stop_resumes_prepared_checkpoint_without_cloud_baseline(self):
        write = activation.write_checkpoint

        def kill_after_stop(path, stage, bot_id, bot_image):
            if stage == "logout-attempted":
                self.assertFalse(self.bot_running)
                os.kill(os.getpid(), signal.SIGKILL)
            return write(path, stage, bot_id, bot_image)

        with mock.patch.object(activation, "write_checkpoint", side_effect=kill_after_stop):
            self.assert_process_killed(self.enable)
        self.assertEqual(self.checkpoint()["stage"], "prepared")
        self.assertEqual(self.checkpoint()["bot_id"], BOT_ID)
        self.assertEqual(self.checkpoint()["bot_image"], BOT_IMAGE)
        self.assert_target_limits()
        self.bot_running = False
        self.current_image = BOT_IMAGE
        self.enable()
        self.assertEqual(len(self.logout_events()), 1)
        self.assertTrue(self.local_auth_events())
        self.assert_no_cloud_restart()
        self.assertEqual(len(self.deploy_observations), 1)
        self.assertTrue(self.deploy_observations[0]["local"])
        self.assertEqual(self.checkpoint()["stage"], "complete")
        self.assertEqual(
            (self.deployment / "shitverter.env.before-local-api").read_text(encoding="utf-8"),
            self.original_env,
        )

    def test_checkpoint_for_another_bot_is_preserved_and_rejected_before_mutation(self):
        self.seed_checkpoint("cloud-logged-out", bot_id=BOT_ID + 1)
        checkpoint = (self.deployment / "local-api.migration.json").read_bytes()
        self.current_image = BOT_IMAGE
        self.bot_running = False
        with self.assertRaises(activation.ActivationError):
            self.enable()
        self.assertFalse(self.bot_running)
        self.assertFalse(self.logout_events())
        self.assertFalse(self.local_auth_events())
        self.assertFalse(self.stopped_commands())
        self.assertFalse(self.deploy_observations)
        self.assertTrue(all(
            event[0] == "command" and event[1][:2] in (["docker", "inspect"], ["docker", "ps"])
            for event in self.events
        ))
        self.assertEqual((self.deployment / "local-api.migration.json").read_bytes(), checkpoint)
        self.assertEqual(self.env_path.read_text(encoding="utf-8"), self.original_env)

    def test_failed_local_deploy_retains_local_marker_and_resumes_without_logout(self):
        def fail_local_deploy(args):
            if (any(Path(value).name == "deploy.sh" for value in args)
                    and (self.deployment / "local-api.enabled").exists()):
                self.assertEqual(self.current_image, BOT_IMAGE)
                self.assert_target_limits()
                raise activation.ActivationError("Local deployment failed")

        self.fail_command = fail_local_deploy
        with self.assertRaises(activation.ActivationError):
            self.enable()
        self.assertEqual(self.checkpoint()["stage"], "local-authenticated")
        self.assertTrue((self.deployment / "local-api.enabled").exists())
        self.assertFalse(self.bot_running)
        self.assertEqual(len(self.logout_events()), 1)
        self.assertFalse(any(
            event[0] == "command" and event[1][:2] == ["docker", "start"]
            for event in self.events
        ))

        self.fail_command = None
        self.events.clear()
        self.deploy_observations.clear()
        self.enable()
        self.assertFalse(self.logout_events())
        self.assert_no_cloud_restart()
        self.assertEqual(self.checkpoint()["stage"], "complete")

    def test_active_webhook_refuses_before_baseline_stop_or_logout_without_leaking_url(self):
        webhook = f"https://hooks.invalid/private/{TOKEN}"
        original_api = self.api

        def webhook_api(base_url, token, method):
            if method == "getWebhookInfo":
                return {"ok": True, "result": {"url": webhook}}
            return original_api(base_url, token, method)

        with mock.patch.object(activation, "bot_api", side_effect=webhook_api):
            with self.assertRaises(activation.ActivationError) as caught:
                self.enable()
        self.assertFalse(self.deploy_observations)
        self.assertFalse(self.stopped_commands())
        self.assertFalse(self.logout_events())
        self.assertTrue(self.bot_running)
        visible = str(caught.exception) + self.stdout.getvalue() + self.stderr.getvalue()
        self.assertNotIn(TOKEN, visible)
        self.assertNotIn(webhook, visible)

    def test_repeat_activation_stays_local_and_preserves_original_backup(self):
        self.enable()
        self.events.clear()
        self.deploy_observations.clear()
        self.enable()
        self.assertFalse(self.logout_events())
        self.assert_no_cloud_restart()
        self.assertEqual(self.checkpoint()["stage"], "complete")
        self.assert_target_limits()
        self.assertEqual(
            (self.deployment / "shitverter.env.before-local-api").read_text(encoding="utf-8"),
            self.original_env,
        )

    def test_environment_update_removes_duplicate_limits_without_touching_other_settings(self):
        self.original_env += "MAX_INPUT_BYTES=1\nMAX_OUTPUT_BYTES=2\n"
        self.env_path.write_text(self.original_env, encoding="utf-8")
        self.enable()
        self.assert_target_limits()

    def test_success_does_not_put_token_or_api_hash_in_commands_output_or_checkpoint(self):
        self.enable()
        observable = (
            repr(self.events)
            + self.stdout.getvalue()
            + self.stderr.getvalue()
            + (self.deployment / "local-api.migration.json").read_text(encoding="utf-8")
            + (self.deployment / "telegram-api-image.env").read_text(encoding="utf-8")
        )
        self.assertNotIn(TOKEN, observable)
        self.assertNotIn(API_HASH, observable)
        saved_credentials = self.deployment / "telegram-api.env"
        self.assertEqual(saved_credentials.stat().st_mode & 0o777, 0o600)


if __name__ == "__main__":
    unittest.main()
