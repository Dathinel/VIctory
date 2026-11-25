import os
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

here = Path(__file__).resolve().parent
rustc, dxc, runner, repo, bucket, decode, lane = (Path(a).resolve() for a in sys.argv[1:])
base = "5b2c0d07b08c5eab275d54e62a481276721991bc"
source = subprocess.check_output(["git", "-C", repo, "show", base + ":crates/victory-formats/src/bcpack.rs"], text=True)

def extract_function(signature,indent):
    return source[source.index(signature):].split("\n"+indent+"}",1)[0]+"\n"+indent+"}"

constants = [line for line in source.splitlines() if line.startswith(("const LOW:", "const ROUND_BYTES:"))]
lane_source = source[source.index("#[derive(",source.index("// Lanes.")):source.index("/// The bucket holding")]
layout = source[source.index("    let (widths, per):"):].split("\n    };",1)[0] + "\n    };"
layout_fn = "fn layout(bc: u8) -> (&'static [u8],usize) {\n" + layout + "\n(widths,per)\n}"
parts = [layout_fn,*constants, lane_source, extract_function("fn bucket_of(",""), extract_function("fn lanes(",""),
         extract_function("fn fragment(width:", "    "), extract_function("fn stream(t:", "    ")]
with TemporaryDirectory() as folder:
    tmp = Path(folder)
    (tmp/"oracle.rs").write_text("\n".join(parts)+"\n"+(here/"cases.rs").read_text())
    subprocess.run([rustc,"--edition=2024","-O","oracle.rs"],cwd=tmp,check=True)
    subprocess.run([tmp/"oracle.exe"],cwd=tmp,check=True)
    for name,header in [("bcpack-bucket",bucket),("bcpack-rans",decode),("bcpack-lane",lane)]:
        (tmp/name).mkdir()
        (tmp/name/header.name).write_bytes(header.read_bytes())
    src=tmp/"schedule"
    src.mkdir()
    (src/"check.hlsl").write_bytes((here/"check.hlsl").read_bytes())
    for mutant in ('','budget','waiting'):
        shader=(here/"lanes.hlsli").read_text()
        if mutant == 'waiting':
            needle = 'bool waiting = lane.symbol != 256 && (done & bit) != 0;'
            assert shader.count(needle) == 1
            shader = shader.replace(needle, needle + '\n            if (waiting) return false;')
        if mutant == 'budget':
            needle = "if (lane.need != 0 && !waiting && taken < 8 && cursor < end)"
            assert shader.count(needle) == 1
            shader=shader.replace(needle, "if (lane.need != 0 && !waiting && taken == 8 && cursor < end) return false;\n            " + needle)
        (src/"lanes.hlsli").write_text(shader)
        subprocess.run([dxc,"-T","cs_6_6","-E","main","-O3","-Fo",tmp/"bc1-records.dxil",src/"check.hlsl"],check=True)
        for case,cut in ([(2,0)] if mutant else [(c,short) for c in range(13) for short in range(2)]):
            for count,i in [(2,0),(260,case)]:
                stem=f"case-{i}-{cut}"
                (tmp/f"input-{count}.bin").write_bytes((tmp/(stem+".in")).read_bytes())
                expected=(tmp/(stem+".out")).read_bytes()
                assert len(expected)<=8*count
                (tmp/f"expected-{count}.bin").write_bytes(expected.ljust(8*count,b"\0"))
            trial=subprocess.run([runner,"--exact","emit::valu_tests::vic117_bc1_gpu","--nocapture"],cwd=tmp,env=dict(os.environ,BC1_TEST_DIR=folder),capture_output=True,text=True)
            log=trial.stdout+trial.stderr
            if mutant:
                assert trial.returncode and "assertion `left == right` failed: hardware: 260" in log.splitlines(),log
                assert "hardware: 2 blocks matched" in log,log
                print(mutant, "control passed one lane and failed nine lanes on hardware.")
            else:
                assert trial.returncode==0 and all(f"{d}: 260 blocks matched" in log for d in ("hardware","warp")),log
                print(f"layout={case} half_input={cut}: status, cursor and output match on hardware and WARP.")
