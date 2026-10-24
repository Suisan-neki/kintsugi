"""Local TLS passthrough to fixed OpenEMR containers without publishing container ports."""
import contextlib
import os
import re
import select
import socket
import socketserver
import subprocess
import threading
import time


# Fixed code and fixed destination, never a request body or a user-supplied command.
# The post-EOF deadline also bounds PHP when a disconnected docker exec client
# leaves the process running inside the container.
PHP_RELAY = r'''
$s = @stream_socket_client("tcp://127.0.0.1:443", $errno, $error, 5);
if (!$s) exit(70);
foreach ([STDIN, STDOUT, $s] as $f) stream_set_blocking($f, false);
$toServer = $toHost = ""; $input = $remote = true; $closed = false;
$deadline = microtime(true) + 300; $idle = microtime(true) + 30;
while (microtime(true) < min($deadline, $idle)) {
    $read = $write = [];
    if ($input && strlen($toServer) < 65536) $read[] = STDIN;
    if ($remote && strlen($toHost) < 65536) $read[] = $s;
    if ($toServer !== "") $write[] = $s;
    if ($toHost !== "") $write[] = STDOUT;
    if (!$read && !$write) break;
    $except = null;
    $n = @stream_select($read, $write, $except, 1);
    if ($n === false) break;
    foreach ($read as $f) {
        $pending = $f === STDIN ? strlen($toServer) : strlen($toHost);
        $chunk = @fread($f, min(16384, 65536 - $pending));
        if ($chunk === false) exit(71);
        if ($chunk !== "") {
            if ($f === STDIN) $toServer .= $chunk; else $toHost .= $chunk;
            $idle = microtime(true) + 30;
        } elseif (feof($f)) {
            if ($f === STDIN) { $input = false; $deadline = min($deadline, microtime(true) + 5); }
            else $remote = false;
        }
    }
    foreach ($write as $f) {
        if ($f === $s) $buffer =& $toServer; else $buffer =& $toHost;
        $written = @fwrite($f, $buffer);
        if ($written === false) exit(72);
        $buffer = substr($buffer, $written); unset($buffer);
        if ($written) $idle = microtime(true) + 30;
    }
    if (!$input && $toServer === "" && !$closed) {
        stream_socket_shutdown($s, STREAM_SHUT_WR); $closed = true;
    }
    if (!$remote && $toHost === "") break;
}
fclose($s);
'''


def relay(client, command, stop, children, lock, *, idle_seconds=30):
    """Copy binary bytes with bounded buffers and half-close support (POSIX pipes)."""
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, bufsize=0)
    with lock:
        children.add(process)
    try:
        client.setblocking(False)
        for pipe in [process.stdin, process.stdout]:
            os.set_blocking(pipe.fileno(), False)
        to_child = to_client = b""
        client_open = child_open = True
        deadline, idle = time.monotonic() + 300, time.monotonic() + idle_seconds
        while not stop.is_set() and time.monotonic() < min(deadline, idle):
            reading = ([client] if client_open and len(to_child) < 65536 else [])
            reading += [process.stdout] if child_open and len(to_client) < 65536 else []
            writing = ([process.stdin] if to_child else []) + ([client] if to_client else [])
            if not reading and not writing:
                break
            readable, writable, _ = select.select(reading, writing, [], 0.25)
            for source in readable:
                chunk = client.recv(min(16384, 65536 - len(to_child))) if source is client else os.read(source.fileno(), min(16384, 65536 - len(to_client)))
                if source is client:
                    to_child += chunk
                    client_open = bool(chunk)
                else:
                    to_client += chunk
                    child_open = bool(chunk)
                if chunk:
                    idle = time.monotonic() + idle_seconds
            for target in writable:
                if target is client:
                    to_client = to_client[client.send(to_client):]
                else:
                    to_child = to_child[os.write(target.fileno(), to_child):]
                idle = time.monotonic() + idle_seconds
            if not client_open and not to_child and not process.stdin.closed:
                process.stdin.close()
            if not child_open and not to_client:
                with contextlib.suppress(OSError):
                    client.shutdown(socket.SHUT_WR)
                break
    except (OSError, ValueError):
        pass  # A closed browser socket or terminated child ends only this connection.
    finally:
        for pipe in [process.stdin, process.stdout]:
            with contextlib.suppress(OSError):
                pipe.close()
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        with lock:
            children.discard(process)


class ForwardPair:
    """Own two loopback listeners and all local relay processes until context exit."""
    def __init__(self, containers):
        if set(containers) != {"A", "B"} or any(not re.fullmatch(r"[0-9a-f]{64}", c) for c in containers.values()):
            raise ValueError("A・Bの固定コンテナIDが必要です")
        self.containers = containers
        self.stop = threading.Event()
        self.capacity = threading.BoundedSemaphore(16)
        self.children, self.servers, self.threads, self.sockets = set(), {}, [], set()
        self.lock = threading.Lock()

    def __enter__(self):
        owner = self

        class Handler(socketserver.BaseRequestHandler):
            def handle(self):
                if not owner.capacity.acquire(blocking=False):
                    return
                with owner.lock:
                    owner.sockets.add(self.request)
                try:
                    relay(self.request, ["docker", "exec", "-i", self.server.container,
                          "php", "-d", "display_errors=stderr", "-r", PHP_RELAY],
                          owner.stop, owner.children, owner.lock)
                finally:
                    with owner.lock:
                        owner.sockets.discard(self.request)
                    owner.capacity.release()

        try:
            for label, container in self.containers.items():
                server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), Handler)
                server.container = container
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                try:
                    thread.start()
                except BaseException:
                    server.server_close()
                    raise
                self.servers[label] = server
                self.threads.append(thread)
            return self
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def port(self, label):
        return self.servers[label].server_address[1]

    def __exit__(self, *_):
        self.stop.set()
        with self.lock:
            for connection in self.sockets:
                with contextlib.suppress(OSError):
                    connection.shutdown(socket.SHUT_RDWR)
            for child in self.children:
                with contextlib.suppress(OSError):
                    child.terminate()
        for server in self.servers.values():
            server.shutdown()
            server.server_close()  # Joins request threads; relay() reaps their child processes.
        for thread in self.threads:
            thread.join()
