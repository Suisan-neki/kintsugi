"""The manual trial must keep both disposable environments and their records separate."""
import importlib.util
import json
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
            self.assertEqual(app["ports"], [{"target": 443, "host_ip": "127.0.0.1", "protocol": "tcp"}])
            self.assertEqual(db["volumes"], ["db-" + label + ":/var/lib/mysql"])
            self.assertNotIn("volumes", app)
            self.assertEqual(app["environment"]["MYSQL_HOST"], "mysql-" + label)
        with self.assertRaises(FileExistsError):
            self.prepare()

    def test_old_engine_with_localhost_publish_bug_is_rejected(self):
        with patch.object(trial.subprocess, "check_output", return_value="27.5.1\n"):
            with self.assertRaisesRegex(ValueError, "28"):
                trial.check_engine()
        with patch.object(trial.subprocess, "check_output", return_value="28.0.0\n"):
            trial.check_engine()

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

    def test_status_never_offers_public_url(self):
        manifest = self.prepare()
        row = {"Service": "openemr-a", "State": "running", "Publishers": [
            {"TargetPort": 443, "PublishedPort": 34567, "URL": "0.0.0.0"}]}
        with patch.object(trial, "docker") as docker:
            docker.return_value.stdout = json.dumps(row)
            with self.assertRaisesRegex(ValueError, "127.0.0.1"):
                trial.status(self.output, manifest)

    def test_ready_urls_have_distinct_hosts_and_no_query_or_credentials(self):
        manifest = self.prepare()
        rows = [{"Service": "openemr-" + label, "State": "running", "Publishers": [
            {"TargetPort": 443, "PublishedPort": port, "URL": "127.0.0.1"}]}
                for label, port in [("a", 34001), ("b", 34002)]]
        with patch.object(trial, "docker") as docker, patch.object(trial.urllib.request, "build_opener") as opener:
            docker.return_value.stdout = json.dumps(rows)
            opener.return_value.open.return_value.__enter__.return_value.status = 200
            result = trial.status(self.output, manifest)
        self.assertEqual(result["state"], "ready")
        self.assertEqual([v["url"] for v in result["variants"]], [
            "https://127.0.0.1:34001/interface/login/login.php",
            "https://localhost:34002/interface/login/login.php"])
        self.assertNotIn(json.loads((self.output / "credentials.json").read_text())["password"], json.dumps(result))

    def test_no_containers_or_stopped_containers_are_not_ready(self):
        for text in ["", "[]", '{"Service":"openemr-a","State":"exited","Publishers":null}']:
            with self.subTest(text=text), patch.object(trial, "docker") as docker:
                manifest = self.prepare() if not self.output.exists() else trial.load(self.output)
                docker.return_value.stdout = text
                result = trial.status(self.output, manifest)
                self.assertEqual(result["state"], "not_ready")
                self.assertTrue(all(v["url"] is None for v in result["variants"]))


if __name__ == "__main__":
    unittest.main()
