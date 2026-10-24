"""Prepare and manage two disposable, local OpenEMR environments for human trials."""
import argparse
import contextlib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import ssl
import subprocess
import sys
import tempfile
import time
import urllib.request


DEFAULT_RECIPE = Path(__file__).resolve().parents[1] / "examples/openemr-browser/soap-recipe.json"
forward_spec = importlib.util.spec_from_file_location("openemr_forward", Path(__file__).with_name("openemr-forward.py"))
forward = importlib.util.module_from_spec(forward_spec)
forward_spec.loader.exec_module(forward)


def write_json(path, value):
    """Publish a complete file; a failed write must not truncate the previous record."""
    fd, temporary = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as output:
            json.dump(value, output, ensure_ascii=False, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        if os.name == "posix":
            directory = os.open(path.parent, os.O_RDONLY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def recipe(path):
    raw = path.read_bytes()
    value = json.loads(raw)
    source = value["source"]
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", source["repository"]) or not re.fullmatch(r"[0-9a-f]{40}", source["commit"]):
        raise ValueError("OpenEMR の固定コミットを指定してください")
    services = value["containers"]["services"]
    if {s["name"] for s in services} != {"mysql", "openemr"} or len(services) != 2:
        raise ValueError("OpenEMR と mysql の2サービスのレシピを指定してください")
    for service in services:
        if not re.fullmatch(r"[^\s$]+@sha256:[0-9a-f]{64}", service["image"]):
            raise ValueError("イメージはダイジェストで固定してください")
        if service.get("web_build"):
            raise ValueError("この試用手順では既にビルドしたイメージを指定してください")
    return value, hashlib.sha256(raw).hexdigest()


def prepare(directory, before_path, after_path, allow_identical=False):
    inputs = [recipe(before_path), recipe(after_path)]
    sources = [value["source"] for value, _ in inputs]
    if sources[0]["repository"] != sources[1]["repository"]:
        raise ValueError("A・Bは同じリポジトリのコミットを指定してください。フォークを使う場合は両方を同じフォークで参照してください")
    apps = [next(s for s in r[0]["containers"]["services"] if s["name"] == "openemr") for r in inputs]
    identical = apps[0]["image"].split("@")[-1] == apps[1]["image"].split("@")[-1]
    if identical and not allow_identical:
        raise ValueError("A・Bが同じイメージです。起動手順だけを確認する場合は --allow-identical を指定してください")
    if sources[0]["commit"] == sources[1]["commit"] and not allow_identical:
        raise ValueError("A・Bは異なるコミットを指定してください。起動手順だけを確認する場合は --allow-identical を指定してください")
    directory.mkdir(mode=0o700, parents=True, exist_ok=False)
    credentials = {"username": "trial-operator", "password": secrets.token_hex(16)}
    root_password, db_password = secrets.token_hex(16), secrets.token_hex(16)
    project = "mugiurushi-trial-" + secrets.token_hex(6)
    compose = {"services": {}, "volumes": {}, "networks": {}}
    variants = []
    for label, (value, digest) in zip(["a", "b"], inputs):
        compose["networks"][label] = {"internal": True}
        compose["volumes"]["db-" + label] = {}
        database = next(s for s in value["containers"]["services"] if s["name"] == "mysql")
        app = next(s for s in value["containers"]["services"] if s["name"] == "openemr")
        # Copy only the pinned images; never accept host mounts, published DB ports,
        # privileged flags, networks or arbitrary runtime options from a recipe.
        compose["services"]["mysql-" + label] = {
            "image": database["image"], "platform": "linux/amd64", "restart": "no",
            "networks": [label], "volumes": ["db-" + label + ":/var/lib/mysql"],
            "environment": {"MYSQL_ROOT_PASSWORD": root_password, "MARIADB_AUTO_UPGRADE": "1"},
            "command": ["mariadbd", "--character-set-server=utf8mb4"],
            "healthcheck": {"test": ["CMD", "/usr/local/bin/healthcheck.sh", "--su-mysql", "--connect", "--innodb_initialized"], "interval": "5s", "timeout": "5s", "retries": 24},
        }
        compose["services"]["openemr-" + label] = {
            "image": app["image"], "platform": "linux/amd64", "restart": "no",
            "networks": [label],
            "depends_on": {"mysql-" + label: {"condition": "service_healthy"}},
            "environment": {"MYSQL_HOST": "mysql-" + label, "MYSQL_ROOT_PASS": root_password,
                            "MYSQL_USER": "openemr", "MYSQL_PASS": db_password,
                            "OE_USER": credentials["username"], "OE_PASS": credentials["password"]},
        }
        variants.append({"label": label.upper(), "source": value["source"], "recipe_sha256": digest,
                         "app_image": app["image"], "database_image": database["image"], "url": None})
    write_json(directory / "compose.json", compose)
    manifest = {"schema_version": 1, "project": project, "state": "prepared", "variants": variants,
                "identical_images": identical, "human_evaluation": "not_conducted",
                "source_image_correspondence": "declared_not_verified",
                "transport": "loopback_tcp_to_docker_exec_php",
                "relay_sha256": hashlib.sha256(Path(forward.__file__).read_bytes()).hexdigest(),
                "data": "Fresh separate databases; enter synthetic data manually in both environments.",
                "compose_sha256": hashlib.sha256((directory / "compose.json").read_bytes()).hexdigest()}
    write_json(directory / "credentials.json", credentials)
    write_json(directory / "trial.json", manifest)
    return manifest


def load(directory):
    manifest = json.loads((directory / "trial.json").read_text())
    if not re.fullmatch(r"mugiurushi-trial-[0-9a-f]{12}", manifest["project"]):
        raise ValueError("試用プロジェクト名が不正です")
    actual = hashlib.sha256((directory / "compose.json").read_bytes()).hexdigest()
    if actual != manifest["compose_sha256"]:
        raise ValueError("生成後に compose.json が変更されています。新しい試用環境を準備してください")
    return manifest


def docker(directory, manifest, *args, capture=False):
    command = ["docker", "compose", "--project-directory", str(directory), "-p", manifest["project"],
               "-f", str(directory / "compose.json"), *args]
    return subprocess.run(command, check=True, text=True, stdout=subprocess.PIPE if capture else None)


def containers(directory, manifest):
    raw = docker(directory, manifest, "ps", "--no-trunc", "--all", "--format", "json", capture=True).stdout.strip()
    # Compose releases have emitted both arrays and one JSON object per line.
    rows = json.loads(raw) if raw.startswith("[") else [json.loads(line) for line in raw.splitlines()]
    result = {}
    for row in rows:
        if any(p.get("PublishedPort") for p in row.get("Publishers") or []):
            raise ValueError("試用コンテナに公開ポートがあります。down で停止してください")
        if row["Service"] in ["openemr-a", "openemr-b"] and row["State"] == "running":
            container = row.get("ID", "")
            if not isinstance(container, str) or not re.fullmatch(r"[0-9a-f]{64}", container):
                raise ValueError(f"Composeが完全なコンテナIDを返していません: service={row['Service']}, IDの文字数={len(str(container))}")
            result[row["Service"][-1].upper()] = container
    return result


def status(directory, manifest, pair=None):
    running = containers(directory, manifest)
    if pair is None:
        return {**manifest, "containers_running": sorted(running), "relay_status": "last_recorded_not_live_probe"}
    for variant in manifest["variants"]:
        variant["ready"] = False
        label = variant["label"]
        if pair.containers.get(label) != running.get(label):
            # A stored URL is valid only while this serve process owns its listener.
            variant["url"] = None
            continue
        base = "https://127.0.0.1:" + str(pair.port(label))
        host = "127.0.0.1" if variant["label"] == "A" else "localhost"
        variant["url"] = f"https://{host}:{pair.port(label)}/interface/login/login.php"
        try:
            # The fixed local image uses a self-signed certificate. No remote URL is accepted.
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                                 urllib.request.HTTPSHandler(context=ssl._create_unverified_context()),
                                                 NoRedirect())
            with opener.open(base + "/meta/health/readyz", timeout=3) as response:
                variant["ready"] = response.status == 200
        except OSError:
            pass
    manifest["state"] = "ready" if all(v["ready"] for v in manifest["variants"]) else "serving"
    write_json(directory / "trial.json", manifest)
    return manifest


@contextlib.contextmanager
def serve_lock(directory):
    path = directory / "serve.lock"
    with path.open("x") as lock:
        try:
            lock.write(str(os.getpid()) + "\n")
            lock.flush()
            yield
        finally:
            path.unlink()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "up", "serve", "status", "down"])
    parser.add_argument("directory", type=Path)
    parser.add_argument("--before-recipe", type=Path, default=DEFAULT_RECIPE)
    parser.add_argument("--after-recipe", type=Path)
    parser.add_argument("--allow-identical", action="store_true")
    args = parser.parse_args()
    directory = args.directory.resolve()
    try:
        if args.command == "prepare":
            if not args.after_recipe:
                parser.error("prepare には --after-recipe が必要です")
            manifest = prepare(directory, args.before_recipe, args.after_recipe, args.allow_identical)
        else:
            manifest = load(directory)
            if args.command == "up":
                if manifest["state"] != "prepared":
                    raise ValueError("up は新規の環境だけで実行できます。再試用には別のディレクトリを準備してください")
                manifest["state"] = "starting"
                write_json(directory / "trial.json", manifest)
                try:
                    docker(directory, manifest, "up", "-d", "--wait", "--wait-timeout", "300")
                except (OSError, ValueError, subprocess.SubprocessError):
                    manifest["state"] = "start_failed"
                    write_json(directory / "trial.json", manifest)
                    print("起動に失敗しました。down で、この試用環境の残った資源を削除してください。", file=sys.stderr)
                    raise
                manifest["state"] = "started"
                write_json(directory / "trial.json", manifest)
                manifest = status(directory, manifest)
            elif args.command == "serve":
                with serve_lock(directory):
                    try:
                        with forward.ForwardPair(containers(directory, manifest)) as pair:
                            deadline = time.monotonic() + 240
                            while status(directory, manifest, pair)["state"] != "ready":
                                if time.monotonic() >= deadline:
                                    raise ValueError("OpenEMRの準備が完了しません。試用を開始せず、down で停止してください")
                                time.sleep(2)
                            print(json.dumps(manifest, ensure_ascii=False, indent=2), flush=True)
                            print("試用中です。終了時は Ctrl-C を押し、down でコンテナとDBを削除してください。", file=sys.stderr)
                            while True:
                                time.sleep(1)
                    except KeyboardInterrupt:
                        pass
                    finally:
                        manifest["state"] = "relay_stopped"
                        for variant in manifest["variants"]:
                            variant.update(url=None, ready=False)
                        write_json(directory / "trial.json", manifest)
            elif args.command == "status":
                manifest = status(directory, manifest)
            else:
                if (directory / "serve.lock").exists():
                    raise ValueError("先に serve を Ctrl-C で終了してください。終了後に down を実行してください")
                docker(directory, manifest, "down", "--volumes", "--remove-orphans")
                manifest["state"] = "removed"
                for variant in manifest["variants"]:
                    variant.update(url=None, ready=False)
                write_json(directory / "trial.json", manifest)
        print(json.dumps(manifest, ensure_ascii=False, indent=2))
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        parser.exit(1, f"{type(error).__name__}: {error}\n")


if __name__ == "__main__":
    main()
