"""The manual trial must keep both disposable environments and their records separate."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("trial", Path(__file__).with_name("openemr-trial.py"))
trial = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trial)
smoke_spec = importlib.util.spec_from_file_location("trial_smoke", Path(__file__).with_name("openemr-trial-smoke.py"))
smoke = importlib.util.module_from_spec(smoke_spec)
smoke_spec.loader.exec_module(smoke)


def boundary_fixture():
    project = "synthetic-trial"
    networks, containers = [], []
    for label in ["a", "b"]:
        for suffix, internal in [(label, True), ("ingress-" + label, False)]:
            networks.append({"Name": project + "-" + suffix, "Internal": internal, "Driver": "bridge",
                             "Labels": {"com.docker.compose.project": project}})
        for kind in ["openemr", "mysql", "proxy"]:
            proxy = kind == "proxy"
            names = [project + "-" + label] + ([project + "-ingress-" + label] if proxy else [])
            ports = {"8443/tcp": [{"HostIp": "127.0.0.1", "HostPort": "34001"}]} if proxy else {}
            containers.append({"Config": {"Labels": {"com.docker.compose.project": project, "com.docker.compose.service": kind + "-" + label},
                                           "User": "101:101" if proxy else "", "Env": ["SYNTHETIC_SECRET=do-not-save"]},
                               "NetworkSettings": {"Networks": {name: {} for name in names}, "Ports": ports},
                               "HostConfig": {"ReadonlyRootfs": proxy, "CapDrop": ["ALL"] if proxy else [], "PortBindings": ports}})
    return containers, networks, project


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
            proxy = compose["services"]["proxy-" + label]
            self.assertEqual(proxy["image"], trial.INGRESS_IMAGE)
            self.assertEqual(proxy["networks"], [label, "ingress-" + label])
            self.assertFalse(compose["networks"]["ingress-" + label]["internal"])
            self.assertEqual(proxy["ports"], [{"target": 8443, "published": "0", "host_ip": "127.0.0.1", "protocol": "tcp"}])
            self.assertEqual(proxy["user"], "101:101")
            self.assertTrue(proxy["read_only"])
            self.assertEqual(proxy["cap_drop"], ["ALL"])
            self.assertEqual(proxy["security_opt"], ["no-new-privileges:true"])
            self.assertNotIn("volumes", proxy)
            self.assertNotIn("environment", proxy)
            self.assertIn("proxy_pass openemr-" + label + ":443;", proxy["command"][0])
            self.assertNotIn("$", proxy["command"][0])
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

    def test_plan_supports_fork_but_requires_different_commits(self):
        value = json.loads(self.after.read_text())
        value["source"]["repository"] = "trial-team/openemr"
        self.after.write_text(json.dumps(value))
        manifest = self.prepare()
        self.assertEqual(manifest["variants"][1]["source"]["repository"], "trial-team/openemr")
        self.output = self.base / "same-commit"
        value["source"] = json.loads(trial.DEFAULT_RECIPE.read_text())["source"]
        self.after.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, "異なるコミット"):
            self.prepare()

    def test_status_never_offers_public_url(self):
        manifest = self.prepare()
        row = {"Service": "proxy-a", "State": "running", "Publishers": [
            {"TargetPort": 8443, "PublishedPort": 34567, "URL": "0.0.0.0"}]}
        with patch.object(trial, "docker") as docker:
            docker.return_value.stdout = json.dumps(row)
            with self.assertRaisesRegex(ValueError, "127.0.0.1"):
                trial.status(self.output, manifest)

    def test_ready_urls_have_distinct_hosts_and_no_query_or_credentials(self):
        manifest = self.prepare()
        rows = [{"Service": "proxy-" + label, "State": "running", "Publishers": [
            {"TargetPort": 8443, "PublishedPort": port, "URL": "127.0.0.1"}]}
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

    def test_missing_publishers_diagnostic_does_not_include_command_or_secrets(self):
        manifest = self.prepare()
        row = {"Service": "proxy-a", "State": "running", "Publishers": None,
               "Command": "MYSQL_PASS=synthetic-secret", "Environment": ["synthetic-secret"]}
        with patch.object(trial, "docker") as docker:
            docker.return_value.stdout = json.dumps(row)
            with self.assertRaisesRegex(ValueError, '"publishers": \\[\\]') as error:
                trial.status(self.output, manifest)
        self.assertIn("proxy-a", str(error.exception))
        self.assertNotIn("synthetic-secret", str(error.exception))
        self.assertNotIn("Command", str(error.exception))

    def test_extra_published_port_is_rejected_even_when_primary_is_local(self):
        manifest = self.prepare()
        row = {"Service": "proxy-a", "State": "running", "Publishers": [
            {"TargetPort": 8443, "PublishedPort": 34567, "URL": "127.0.0.1"},
            {"TargetPort": 80, "PublishedPort": 34568, "URL": "0.0.0.0"}]}
        with patch.object(trial, "docker") as docker:
            docker.return_value.stdout = json.dumps(row)
            with self.assertRaises(ValueError):
                trial.status(self.output, manifest)

    def test_no_containers_or_stopped_containers_are_not_ready(self):
        for text in ["", "[]", '{"Service":"openemr-a","State":"exited","Publishers":null}']:
            with self.subTest(text=text), patch.object(trial, "docker") as docker:
                manifest = self.prepare() if not self.output.exists() else trial.load(self.output)
                manifest["state"] = "starting"
                docker.return_value.stdout = text
                result = trial.status(self.output, manifest)
                self.assertEqual(result["state"], "not_ready")
                self.assertTrue(all(v["url"] is None for v in result["variants"]))

    def test_inspection_does_not_prevent_first_start_or_revive_removed_environment(self):
        manifest = self.prepare()
        for state in ["prepared", "removed"]:
            with self.subTest(state=state), patch.object(trial, "docker") as docker:
                manifest["state"] = state
                docker.return_value.stdout = "[]"
                result = trial.status(self.output, manifest)
                self.assertEqual(result["state"], state)
                self.assertEqual(trial.load(self.output)["state"], state)
                self.assertTrue(all(v["url"] is None and not v["ready"] for v in result["variants"]))

    def test_start_after_prepared_inspection_still_checks_engine_and_starts(self):
        manifest = self.prepare()
        with patch.object(trial, "docker") as docker:
            docker.return_value.stdout = "[]"
            trial.status(self.output, manifest)
        with patch.object(trial, "docker") as docker, patch.object(trial, "check_engine") as check_engine, \
                patch.object(trial.sys, "argv", ["openemr-trial.py", "up", str(self.output)]), patch("builtins.print"):
            docker.return_value.stdout = "[]"
            trial.main()
            check_engine.assert_called_once()
            self.assertEqual(docker.call_args_list[0].args[2], "up")
        self.assertEqual(trial.load(self.output)["state"], "not_ready")

    def test_inspection_reports_unexpected_containers_in_prepared_state(self):
        manifest = self.prepare()
        with patch.object(trial, "docker") as docker:
            docker.return_value.stdout = '{"Service":"openemr-a","State":"exited","Publishers":null}'
            result = trial.status(self.output, manifest)
        self.assertEqual(result["state"], "not_ready")

    def test_actual_boundary_validator_rejects_weakened_app_or_proxy_networks(self):
        fixture = boundary_fixture()
        record = smoke.boundary_record(*fixture)
        self.assertEqual(len(record), 6)
        self.assertNotIn("SYNTHETIC_SECRET", json.dumps(record))
        for change in [
            lambda c, n: c[0]["NetworkSettings"]["Networks"].update({"synthetic-trial-ingress-a": {}}),
            lambda c, n: c[1]["NetworkSettings"]["Ports"].update({"3306/tcp": [{"HostIp": "127.0.0.1", "HostPort": "34001"}]}),
            lambda c, n: c[2]["NetworkSettings"]["Ports"]["8443/tcp"][0].update(HostIp="0.0.0.0"),
            lambda c, n: c[2]["Config"].update(User="0"),
            lambda c, n: n[0].update(Internal=False),
        ]:
            with self.subTest(change=change):
                containers, networks, project = copy.deepcopy(fixture)
                change(containers, networks)
                with self.assertRaises(AssertionError):
                    smoke.boundary_record(containers, networks, project)

    def test_proxy_upstream_cannot_be_chosen_from_repository_input(self):
        with self.assertRaises(ValueError):
            trial.ingress_command("a; proxy_pass external.invalid:443")
        self.assertIn("proxy_pass openemr-a:443;", trial.ingress_command("a"))


if __name__ == "__main__":
    unittest.main()
