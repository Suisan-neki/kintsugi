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
            raise RuntimeError("OpenEMR A/B readiness timed out")
        time.sleep(2)
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
print(json.dumps({"startup": "passed", "login_forms": "passed", "cleanup": "passed",
                  "human_evaluation": "not_conducted", "improvement_comparison": "not_conducted"}))
