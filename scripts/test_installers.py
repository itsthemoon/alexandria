#!/usr/bin/env python3
import hashlib
import io
import os
import pty
import select
import shutil
import signal
import sys
import tarfile
import tempfile
import time


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    installer = os.path.join(root, "site", "install.sh")
    T = tempfile.mkdtemp(prefix="alex-inst-prompt-")
    try:
        home = os.path.join(T, "home")
        dest = os.path.join(T, "chosen")
        R = os.path.join(T, "R")
        for d in (home, dest, R):
            os.makedirs(d)

        tar_path = os.path.join(R, "Alexandria-macos.tar.gz")
        script = b"#!/bin/sh\necho 'alex $1 ok'\n"
        with tarfile.open(tar_path, "w:gz") as tf:
            ti = tarfile.TarInfo("Alexandria.app/Contents/MacOS/alex")
            ti.size = len(script)
            ti.mode = 0o755
            tf.addfile(ti, io.BytesIO(script))

        data = os.urandom(10000)
        with open(os.path.join(R, "small.bin"), "wb") as f:
            f.write(data)
        sha = hashlib.sha256(data).hexdigest()
        base = "file://" + R
        with open(os.path.join(R, "full-FILES.txt"), "w") as f:
            f.write("small.bin %d %s %s/small.bin\n" % (len(data), sha, base))

        env = dict(os.environ)
        env["HOME"] = home
        env["ALEXANDRIA_BASE"] = base
        env["ALEXANDRIA_EDITION"] = "full"
        env.pop("ALEXANDRIA_DIR", None)

        pid, fd = pty.fork()
        if pid == 0:
            devnull = os.open(os.devnull, os.O_RDONLY)
            os.dup2(devnull, 0)
            os.close(devnull)
            os.execve("/bin/bash", ["bash", installer], env)
            os._exit(127)

        out = b""
        sent = False
        deadline = time.time() + 120
        while True:
            try:
                r, _, _ = select.select([fd], [], [], 1.0)
            except (OSError, ValueError):
                r = []
            if r:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:
                    chunk = b""
                if not chunk:
                    break
                out += chunk
                if not sent and b"Where should the library go" in out:
                    os.write(fd, (dest + "\n").encode())
                    sent = True
            else:
                wpid, _ = os.waitpid(pid, os.WNOHANG)
                if wpid == pid:
                    break
                if time.time() > deadline:
                    os.kill(pid, signal.SIGKILL)
                    raise SystemExit("installer timed out:\n%s" % out.decode(errors="replace"))
        os.waitpid(pid, 0)

        if not sent:
            raise SystemExit("prompt was never shown:\n%s" % out.decode(errors="replace"))
        if not os.path.exists(os.path.join(dest, "Alexandria", "small.bin")):
            raise SystemExit("typed folder was not used:\n%s" % out.decode(errors="replace"))
        if os.path.exists(os.path.join(home, "Alexandria")):
            raise SystemExit("library went to the default $HOME instead of the typed folder:\n%s" % out.decode(errors="replace"))
        print("installer prompts for a folder through a pipe with a pty: ok")
    finally:
        shutil.rmtree(T, ignore_errors=True)


if __name__ == "__main__":
    main()
