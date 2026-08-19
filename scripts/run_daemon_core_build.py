#!/usr/bin/env -S python3 -I -S -B
import os, stat, sys; from pathlib import Path
def capture(path):
    handle = os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW), "rb"); before = os.fstat(handle.fileno()); payload = handle.read(); after = os.fstat(handle.fileno()); handle.close()
    if not stat.S_ISREG(before.st_mode) or stat.S_IMODE(before.st_mode) != 0o755 or (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns): raise ValueError("captured build source changed or has an invalid type or mode")
    return payload
launcher = Path(__file__).resolve(); source = launcher.with_name("generate_daemon_core_build.py"); launcher_source, generator_source = globals().get("_EXECUTED_LAUNCHER_SOURCE"), capture(source)
sys.argv = [str(source), *sys.argv[1:]]; exec(compile(generator_source, str(source), "exec"), {"__name__": "__main__", "__file__": str(source), "_EXECUTED_LAUNCHER_SOURCE": launcher_source, "_EXECUTED_SOURCE": generator_source})
