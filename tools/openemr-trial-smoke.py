"""Real Docker startup/handoff/cleanup check, using identical OSS versions only."""
import importlib.util
import json
from pathlib import Path
import subprocess
import time
import urllib.request


spec = importlib.util.spec_from_file_location("trial", Path(__file__).with_name("openemr-trial.py"))
trial = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trial)

def boundary_record(containers, networks, project):
    """Validate inspect results and return only fields safe to save, never Env."""
    expected = {name + "-" + label for label in ["a", "b"] for name in ["openemr", "mysql", "proxy"]}
    by_service = {}
    for container in containers:
        labels = container["Config"]["Labels"]
        assert labels["com.docker.compose.project"] == project, "Unexpected trial project"
        service = labels["com.docker.compose.service"]
        assert service not in by_service, "Duplicate trial service"
        by_service[service] = container
    assert set(by_service) == expected, "Missing or unexpected trial service"
    by_network = {network["Name"]: network for network in networks}
    records = []
    internal_names = []
    for label in ["a", "b"]:
        private = set(by_service["openemr-" + label]["NetworkSettings"]["Networks"])
        assert len(private) == 1, "OpenEMR must use one internal-only network"
        private_name = next(iter(private))
        internal_names.append(private_name)
        assert by_network[private_name]["Internal"] is True, "OpenEMR network is not internal"
        for name in ["openemr", "mysql", "proxy"]:
            service = name + "-" + label
            container = by_service[service]
            connected = set(container["NetworkSettings"]["Networks"])
            ports = container["NetworkSettings"].get("Ports") or {}
            published = {key: bindings for key, bindings in ports.items() if bindings}
            if name == "proxy":
                assert private.issubset(connected) and len(connected) == 2, "Proxy network boundary changed"
                external = next(iter(connected - private))
                assert by_network[external]["Internal"] is False and by_network[external]["Driver"] == "bridge", "Proxy ingress is not a bridge"
                assert set(published) == {"8443/tcp"}, "Unexpected published proxy port"
                bindings = published["8443/tcp"]
                assert len(bindings) == 1 and bindings[0]["HostIp"] == "127.0.0.1", "Proxy is publicly exposed"
                assert 0 < int(bindings[0]["HostPort"]) <= 65535, "Proxy host port was not assigned"
                assert container["Config"]["User"] == "101:101", "Proxy must run without root"
                assert container["HostConfig"]["ReadonlyRootfs"] is True, "Proxy root filesystem is writable"
                assert container["HostConfig"]["CapDrop"] == ["ALL"], "Proxy capabilities were not dropped"
            else:
                assert connected == private, "App or DB has an external route"
                assert not published and not any((container["HostConfig"].get("PortBindings") or {}).values()), "App or DB publishes a host port"
            for network_name in connected:
                assert by_network[network_name]["Labels"]["com.docker.compose.project"] == project, "Unexpected network owner"
            records.append({"service": service,
                            "networks": [{"name": network_name, "internal": by_network[network_name]["Internal"]}
                                         for network_name in sorted(connected)],
                            "published": published})
    assert len(set(internal_names)) == 2, "A and B share a private network"
    return records


def inspect_boundaries(output, manifest):
    ids = trial.docker(output, manifest, "ps", "-q", capture=True).stdout.split()
    assert len(ids) == 6, "Expected two apps, two databases and two ingress proxies"
    containers = json.loads(subprocess.check_output(["docker", "inspect", *ids], text=True))
    names = sorted({name for container in containers for name in container["NetworkSettings"]["Networks"]})
    networks = json.loads(subprocess.check_output(["docker", "network", "inspect", *names], text=True))
    # Only the validated safe subset is written, not the raw inspect with Env.
    record = boundary_record(containers, networks, manifest["project"])
    trial.write_json(output / "boundaries.json", record)


def main():
    output = Path(__file__).resolve().parents[1] / "evaluations/openemr-manual-trial"
    manifest = trial.prepare(output, trial.DEFAULT_RECIPE, trial.DEFAULT_RECIPE, allow_identical=True)
    try:
        trial.check_engine()
        trial.docker(output, manifest, "up", "-d", "--wait", "--wait-timeout", "300")
        deadline = time.monotonic() + 240
        while True:
            manifest = trial.status(output, manifest)
            if manifest["state"] == "ready":
                break
            if time.monotonic() >= deadline:
                raise RuntimeError("OpenEMR A/B readiness timed out: " + json.dumps([v.get("ingress") for v in manifest["variants"]]))
            time.sleep(2)
        inspect_boundaries(output, manifest)
        for variant in manifest["variants"]:
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                                 urllib.request.HTTPSHandler(context=trial.ssl._create_unverified_context()),
                                                 trial.NoRedirect())
            with opener.open(variant["url"], timeout=15) as response:
                page = response.read(1024 * 1024).decode()
                assert response.status == 200 and 'id="authUser"' in page, "OpenEMR login form was not served"
        assert manifest["identical_images"] and manifest["human_evaluation"] == "not_conducted"
        assert manifest["variants"][0]["url"].startswith("https://127.0.0.1:")
        assert manifest["variants"][1]["url"].startswith("https://localhost:")
        trial.write_json(output / "ready.json", manifest)
    finally:
        trial.docker(output, manifest, "down", "--volumes", "--remove-orphans")
        # Verify removal, instead of treating a successful down invocation as proof.
        label = "label=com.docker.compose.project=" + manifest["project"]
        for arguments in [["ps", "-aq"], ["volume", "ls", "-q"], ["network", "ls", "-q"]]:
            result = subprocess.check_output(["docker", *arguments, "--filter", label], text=True)
            assert not result.strip(), "Trial resource remained after cleanup: " + result
        manifest["state"] = "removed"
        for variant in manifest["variants"]:
            variant.update(url=None, ready=False)
        trial.write_json(output / "trial.json", manifest)
    print(json.dumps({"startup": "passed", "login_forms": "passed", "boundaries": "passed", "cleanup": "passed",
                      "human_evaluation": "not_conducted", "improvement_comparison": "not_conducted"}))


if __name__ == "__main__":
    main()
