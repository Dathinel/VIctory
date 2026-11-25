import os
import re
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

here = Path(__file__).resolve().parent
rustc, dxc, runner, repo = (Path(p).resolve() for p in sys.argv[1:])
base = "5b2c0d07b08c5eab275d54e62a481276721991bc"
source = subprocess.check_output(["git", "-C", repo, "show", base + ":crates/victory-formats/src/bcpack.rs"], text=True)
functions = []
for name in ("bc1_color", "bc4_alpha", "bc3_pack"):
    start = source.index(f"fn {name}(")
    functions.append(source[start:source.index("\n}", start) + 2])
plain = source[source.index("struct Plain {"):].split("\n}", 1)[0] + "\n}"
constants = [line for line in source.splitlines() if line.startswith(("const BC1_CODE:", "const BC4_CODE:"))]

with TemporaryDirectory() as folder:
    tmp = Path(folder)
    oracle = "\n".join(["#[derive(Default)]", plain, *constants, *functions, (here/"cases.rs").read_text()])
    (tmp/"oracle.rs").write_text(oracle)
    subprocess.run([rustc,"--edition=2024","oracle.rs"],cwd=tmp,check=True)
    subprocess.run([tmp/"oracle.exe"],cwd=tmp,check=True)
    for stage in ("bc3","bc4","alpha-carry","color-offset"):
        shader = here.parent/"bc4-alpha.hlsl" if stage == "bc4" else here/"pack.hlsl"
        if stage in ("alpha-carry","color-offset"):
            code = shader.read_text()
            if stage == "color-offset":
                assert code.count("base + 24 +") == 1
                code = code.replace("base + 24 +","base + 8 +")
            else:
                header = (here.parent/"bc-alpha-pack.hlsli").read_text()
                carry = "if (shift == 31) packed.y |= code >> 1;"
                assert header.count(carry) == 1
                (tmp/"altered.hlsli").write_text(header.replace(carry,""))
                code = code.replace("../bc-alpha-pack.hlsli","altered.hlsli")
            shader = tmp/"altered.hlsl"
            shader.write_text(code)
        subprocess.run([dxc,"-T","cs_6_6","-E","main","-O3","-I",here,"-Fo",tmp/"bc1-records.dxil",shader],check=True)
        for count in ([0,1,64,65,449] if stage in ("bc3","bc4") else [1]):
            for slot,n in [(2,0),(260,count)]:
                prefix = "alpha-" if stage == "bc4" else ""
                data = (tmp/f"{prefix}input-{n}.bin").read_bytes()
                expected = (tmp/f"{prefix}expected-{n}.bin").read_bytes()
                assert len(expected) == (8 if stage == "bc4" else 16) * n
                (tmp/f"input-{slot}.bin").write_bytes(data)
                (tmp/f"expected-{slot}.bin").write_bytes(expected)
            trial = subprocess.run([runner,"--exact","emit::valu_tests::bc4_sized_gpu","--ignored","--nocapture"],cwd=tmp,env=dict(os.environ,BC1_TEST_DIR=folder),capture_output=True,text=True)
            log = trial.stdout + trial.stderr
            if stage in ("alpha-carry","color-offset"):
                difference = re.search(r"hardware: mismatch in slot 260 at byte (\d+)",log)
                assert trial.returncode and difference and int(difference[1]) == {"alpha-carry":4,"color-offset":12}[stage],log
                print(stage,"control failed at byte",difference[1])
            else:
                assert trial.returncode == 0 and all(f"{backend}: slot 260 matched {len(expected)} bytes and guard" in log for backend in ("hardware","warp")),log
                print(stage,count,"blocks:",len(expected),"bytes matched on hardware and WARP")
