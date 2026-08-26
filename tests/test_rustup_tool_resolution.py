import json, subprocess, sys, tempfile, unittest
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]; GENERATOR = ROOT / "scripts/generate_daemon_core_build.py"; LAUNCHER = ROOT / "scripts/run_daemon_core_build.py"
class RustupToolResolutionTests(unittest.TestCase):
    def test_symlink_proxies_resolve_to_selected_tools(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory); tools = {name: directory / f"real-{name}" for name in ("cargo", "rustc")}
            for name, tool in tools.items(): tool.write_text(f"#!/bin/sh\nprintf '%s\\n' '{name}'\n"); tool.chmod(0o755)
            rustup = directory / "rustup"; rustup.write_text('#!/bin/sh\nif [ "$1" = "which" ]; then printf \'%s/real-%s\\n\' "${0%/*}" "$2"; exit 0; fi\nexit 2\n'); rustup.chmod(0o755); proxies = {name: directory / name for name in tools}
            for proxy in proxies.values(): proxy.symlink_to(rustup.name)
            program = 'import json,pathlib,sys\ns=pathlib.Path(sys.argv[1]);l=pathlib.Path(sys.argv[2]);b=s.read_bytes();n={"__name__":"probe","__file__":str(s),"_EXECUTED_SOURCE":b,"_EXECUTED_LAUNCHER_SOURCE":l.read_bytes()};exec(compile(b,str(s),"exec"),n);n["shutil"].which=lambda name:str(pathlib.Path(sys.argv[3])/name);p=n["resolve_tools"](pathlib.Path(sys.argv[4]));print(json.dumps({name:str(p[name]) for name in ("cargo","rustc")},sort_keys=True))\n'
            result = subprocess.run([sys.executable, "-I", "-S", "-B", "-c", program, GENERATOR, LAUNCHER, directory, ROOT], cwd=ROOT, capture_output=True, text=True); self.assertEqual(result.returncode, 0, result.stderr); paths = json.loads(result.stdout)
            for name, tool in tools.items(): self.assertEqual(Path(paths[name]), tool.resolve()); self.assertEqual(subprocess.run([paths[name]], cwd="/", env={"PATH": "/usr/bin:/bin"}, capture_output=True, text=True).stdout.strip(), name)
if __name__ == "__main__": unittest.main()
