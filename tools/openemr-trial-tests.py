"""The manual trial must keep both disposable environments and their records separate."""
import importlib.util
import json
import socket
import sys
import threading
import time
from types import SimpleNamespace
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("trial", Path(__file__).with_name("openemr-trial.py"))
trial = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trial)


class TrialTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.after = self.base / "after.json"
        value = json.loads(trial.DEFAULT_RECIPE.read_text())
        value["source"]["commit"] = "b" * 40
        next(s for s in value["containers"]["services"] if s["name"] == "openemr")["image"] = "reviewed/openemr@sha256:" + "b" * 64
        self.after.write_text(json.dumps(value))
        self.output = self.base / "pair"

    def prepare(self):
        return trial.prepare(self.output, trial.DEFAULT_RECIPE, self.after)

    def test_distinct_databases_local_ports_and_no_host_mounts(self):
        manifest = self.prepare()
        compose = json.loads((self.output / "compose.json").read_text())
        self.assertFalse(manifest["identical_images"])
        self.assertEqual(manifest["human_evaluation"], "not_conducted")
        self.assertEqual(manifest["source_image_correspondence"], "declared_not_verified")
        self.assertEqual(manifest["variants"][1]["source"]["commit"], "b" * 40)
        for label in ["a", "b"]:
            db, app = [compose["services"][name + "-" + label] for name in ["mysql", "openemr"]]
            self.assertEqual(db["networks"], [label])
            self.assertEqual(app["networks"], [label])
            self.assertTrue(compose["networks"][label]["internal"])
            self.assertNotIn("ports", db)
            self.assertNotIn("ports", app)
            self.assertEqual(db["volumes"], ["db-" + label + ":/var/lib/mysql"])
            self.assertNotIn("volumes", app)
            self.assertEqual(app["environment"]["MYSQL_HOST"], "mysql-" + label)
        with self.assertRaises(FileExistsError):
            self.prepare()

    def test_identical_images_need_explicit_setup_only_flag(self):
        with self.assertRaisesRegex(ValueError, "同じイメージ"):
            trial.prepare(self.output, trial.DEFAULT_RECIPE, trial.DEFAULT_RECIPE)
        manifest = trial.prepare(self.output, trial.DEFAULT_RECIPE, trial.DEFAULT_RECIPE, True)
        self.assertTrue(manifest["identical_images"])

    def test_changed_or_unpinned_configuration_cannot_be_started(self):
        self.prepare()
        (self.output / "compose.json").write_text("{}")
        with self.assertRaisesRegex(ValueError, "変更されています"):
            trial.load(self.output)
        value = json.loads(self.after.read_text())
        value["containers"]["services"][1]["image"] = "openemr/openemr:latest"
        self.after.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, "ダイジェスト"):
            trial.recipe(self.after)

    def test_plan_requires_same_repository_and_different_commits(self):
        value = json.loads(self.after.read_text())
        value["source"]["repository"] = "trial-team/openemr"
        self.after.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, "同じリポジトリ"):
            self.prepare()
        value["source"] = json.loads(trial.DEFAULT_RECIPE.read_text())["source"]
        self.after.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, "異なるコミット"):
            self.prepare()

    def test_status_rejects_all_published_container_ports(self):
        manifest = self.prepare()
        row = {"Service": "openemr-a", "State": "running", "Publishers": [
            {"TargetPort": 443, "PublishedPort": 34567, "URL": "127.0.0.1"}]}
        with patch.object(trial, "docker") as docker:
            docker.return_value.stdout = json.dumps(row)
            with self.assertRaisesRegex(ValueError, "公開ポート"):
                trial.status(self.output, manifest)

    def test_compose_ids_are_requested_untruncated_and_short_ids_are_rejected(self):
        manifest = self.prepare()
        row = {"Service": "openemr-a", "State": "running", "Publishers": None, "ID": "a" * 64}
        with patch.object(trial, "docker") as docker:
            docker.return_value.stdout = json.dumps(row)
            self.assertEqual(trial.containers(self.output, manifest), {"A": "a" * 64})
            self.assertIn("--no-trunc", docker.call_args.args)
            row["ID"] = "a" * 12
            docker.return_value.stdout = json.dumps(row)
            with self.assertRaisesRegex(ValueError, "service=openemr-a, IDの文字数=12"):
                trial.containers(self.output, manifest)

    def test_ready_urls_require_owned_forwarder_and_status_does_not_erase_them(self):
        manifest = self.prepare()
        rows = [{"Service": "openemr-" + label, "ID": label * 64, "State": "running", "Publishers": None}
                for label in ["a", "b"]]
        pair = SimpleNamespace(containers={"A": "a" * 64, "B": "b" * 64}, port=lambda label: 34001 if label == "A" else 34002)
        with patch.object(trial, "docker") as docker, patch.object(trial.urllib.request, "build_opener") as opener:
            docker.return_value.stdout = json.dumps(rows)
            opener.return_value.open.return_value.__enter__.return_value.status = 200
            result = trial.status(self.output, manifest, pair)
            saved = (self.output / "trial.json").read_bytes()
            inspection = trial.status(self.output, result)
        self.assertEqual(result["state"], "ready")
        self.assertEqual([v["url"] for v in result["variants"]], [
            "https://127.0.0.1:34001/interface/login/login.php",
            "https://localhost:34002/interface/login/login.php"])
        self.assertEqual((self.output / "trial.json").read_bytes(), saved)
        self.assertEqual(inspection["relay_status"], "last_recorded_not_live_probe")
        self.assertNotIn(json.loads((self.output / "credentials.json").read_text())["password"], json.dumps(result))

    def test_no_running_containers_never_creates_trial_url(self):
        manifest = self.prepare()
        with patch.object(trial, "docker") as docker:
            docker.return_value.stdout = "[]"
            result = trial.status(self.output, manifest)
        self.assertEqual(result["containers_running"], [])
        self.assertTrue(all(v["url"] is None for v in result["variants"]))

    def test_only_one_serve_can_own_the_trial(self):
        self.prepare()
        with trial.serve_lock(self.output):
            with self.assertRaises(FileExistsError):
                with trial.serve_lock(self.output):
                    self.fail("second owner was allowed")
        self.assertFalse((self.output / "serve.lock").exists())


class ForwardTest(unittest.TestCase):
    def test_idle_connection_releases_subprocess(self):
        client, server = socket.socketpair()
        self.addCleanup(client.close)
        self.addCleanup(server.close)
        stop, children, lock = threading.Event(), set(), threading.Lock()
        worker = threading.Thread(target=trial.forward.relay, args=(server,
            [sys.executable, "-c", "import sys; sys.stdin.buffer.read()"], stop, children, lock),
            kwargs={"idle_seconds": 0.1})
        worker.start()
        worker.join(3)
        self.assertFalse(worker.is_alive())
        self.assertEqual(children, set())

    def test_binary_large_response_after_client_half_close(self):
        client, server = socket.socketpair()
        self.addCleanup(client.close)
        self.addCleanup(server.close)
        client.settimeout(10)
        stop, children, lock = threading.Event(), set(), threading.Lock()
        command = [sys.executable, "-c", "import sys; data=sys.stdin.buffer.read(); sys.stdout.buffer.write(data[::-1])"]
        worker = threading.Thread(target=trial.forward.relay, args=(server, command, stop, children, lock))
        worker.start()
        payload = bytes(range(256)) * 4096
        client.sendall(payload)
        client.shutdown(socket.SHUT_WR)
        result = bytearray()
        while chunk := client.recv(32768):
            result.extend(chunk)
        worker.join(5)
        self.assertFalse(worker.is_alive())
        self.assertEqual(bytes(result), payload[::-1])
        self.assertEqual(children, set())

    def test_context_exit_closes_active_connection_and_reaps_child(self):
        original = trial.forward.subprocess.Popen
        processes = []
        def stand_in(*args, **kwargs):
            process = original([sys.executable, "-c", "import sys; sys.stdin.buffer.read()"], **kwargs)
            processes.append(process)
            return process
        with patch.object(trial.forward.subprocess, "Popen", side_effect=stand_in):
            with trial.forward.ForwardPair({"A": "a" * 64, "B": "b" * 64}) as pair:
                client = socket.create_connection(("127.0.0.1", pair.port("A")), timeout=3)
                self.addCleanup(client.close)
                client.sendall(b"unfinished request")
                deadline = time.monotonic() + 3
                while not processes and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(processes)
            self.assertEqual(pair.children, set())
            self.assertTrue(all(p.poll() is not None for p in processes))
            self.assertTrue(all(not t.is_alive() for t in pair.threads))
            self.assertEqual(client.recv(1), b"")

    def test_connection_limit_rejects_excess_without_starting_child(self):
        original = trial.forward.subprocess.Popen
        processes = []
        def stand_in(*args, **kwargs):
            process = original([sys.executable, "-c", "import sys; sys.stdin.buffer.read()"], **kwargs)
            processes.append(process)
            return process
        with patch.object(trial.forward.subprocess, "Popen", side_effect=stand_in):
            with trial.forward.ForwardPair({"A": "a" * 64, "B": "b" * 64}) as pair:
                pair.capacity = threading.BoundedSemaphore(1)
                first = socket.create_connection(("127.0.0.1", pair.port("A")), timeout=3)
                self.addCleanup(first.close)
                deadline = time.monotonic() + 3
                while not processes and time.monotonic() < deadline:
                    time.sleep(0.01)
                with socket.create_connection(("127.0.0.1", pair.port("B")), timeout=3) as excess:
                    self.assertEqual(excess.recv(1), b"")
                self.assertEqual(len(processes), 1)


if __name__ == "__main__":
    unittest.main()
