import os
import struct
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

here = Path(__file__).resolve().parent
rustc, dxc, runner, repo, bucket, decode = (Path(a).resolve() for a in sys.argv[1:])
base = "5b2c0d07b08c5eab275d54e62a481276721991bc"
source = subprocess.check_output(["git", "-C", repo, "show", base + ":crates/victory-formats/src/bcpack.rs"], text=True)
fragment = source[source.index("    fn fragment(width:"):].split("\n    }", 1)[0] + "\n}"
driver = """
use std::io::{self,Write};

fn main(){
 let mut out=io::BufWriter::new(io::stdout().lock());
 for i in 0..260u32 {
  let width=(i%8) as u8;
  let n=[0,1,2,7,31,128,257,513,1024][(i as usize/8+i as usize%8)%9];
  let mut rng=i+913;
  let symbols:Vec<_>=(0..n).map(|k| {
    rng=rng.wrapping_mul(1664525).wrapping_add(1013904223);
    if k%5 == 1 {0} else {((rng>>16)&((2u32<<width)-1)) as u8}
  }).collect();
  let encoded=fragment(width,&symbols);
  for v in [width as u32,n,encoded.len() as u32]{out.write_all(&v.to_le_bytes()).unwrap();}
  out.write_all(&symbols).unwrap();out.write_all(&encoded).unwrap();
 }
}
"""
def input_and_expected(cases):
    data = bytearray(4 + 16 * len(cases))
    struct.pack_into("<I", data, 0, len(cases))
    rows = []
    for width in range(8):
        rows.append(len(data))
        f = 1 << (15-width)
        starts = [0]+[f << j for j in range(width+1)]
        data.extend(struct.pack("<10I", *(starts + [65536]*(10-len(starts)))))
        data.extend(sum(s<=w*256 for s in starts[1:]) for w in range(256))
        data.extend(struct.pack("<9I", *([f]*len(starts)+[0]*(9-len(starts)))))
    expected = bytearray()
    for i,(width,symbols,payload) in enumerate(cases):
        data.extend(bytes([0xA5]) * (i % 4))
        cursor=len(data)
        data.extend(payload)
        data.extend(bytes(-len(data)%4))
        truth=len(data)
        data.extend(struct.pack("<I",len(symbols))+symbols)
        data.extend(bytes(-len(data)%4))
        struct.pack_into("<4I",data,4+16*i,rows[width],cursor,len(payload),truth)
        expected.extend(struct.pack("<II",len(symbols),1))
    return data,expected

with TemporaryDirectory() as folder:
    tmp=Path(folder)
    low = next(line for line in source.splitlines() if line.startswith("const LOW:"))
    (tmp/"encode.rs").write_text(low+"\n"+fragment+driver)
    subprocess.run([rustc,"-O","encode.rs"],cwd=tmp,check=True)
    records=subprocess.check_output([tmp/"encode.exe"])
    cases=[]
    at=short=long=0
    while at<len(records):
        width,n,size=struct.unpack_from("<III",records,at)
        at+=12
        symbols=records[at:at+n]
        raw=records[at+n:at+n+size]
        at+=n+size
        if raw[0]&32:
            declared=257+int.from_bytes(raw[1:3],"little")
            payload=raw[3:]
            long+=1
        else:
            declared=raw[1]+1
            payload=raw[2:]
            short+=1
        assert len(payload)==declared
        cases.append((width,symbols,payload))
    assert len(cases)==260 and short and long and at==len(records)
    for count in (2,260):
        data,want=input_and_expected(cases[:count])
        (tmp/f"input-{count}.bin").write_bytes(data)
        (tmp/f"expected-{count}.bin").write_bytes(want)
    for name,header in [("bcpack-bucket",bucket),("bcpack-rans",decode)]:
        (tmp/name).mkdir()
        (tmp/name/header.name).write_bytes(header.read_bytes())
    src=tmp/"chain"
    src.mkdir()
    (src/"check.hlsl").write_bytes((here/"check.hlsl").read_bytes())
    for mutate in (False,True):
        shader=(here/"next-symbol.hlsli").read_text()
        if mutate:
            assert shader.count("state = result.y;")==1
            shader=shader.replace("    symbol = 0;", "    bool subsequent = state != 0;\n    symbol = 0;")
            shader=shader.replace("state = result.y;","state = subsequent ? 0 : result.y;")
        (src/"next-symbol.hlsli").write_text(shader)
        subprocess.run([dxc,"-T","cs_6_6","-E","main","-O3","-Fo",tmp/"bc1-records.dxil",src/"check.hlsl"],check=True)
        trial=subprocess.run([runner,"--exact","emit::valu_tests::vic117_bc1_gpu","--nocapture"],cwd=tmp,env=dict(os.environ,BC1_TEST_DIR=folder),capture_output=True,text=True)
        log=trial.stdout+trial.stderr
        if mutate:
            assert trial.returncode and "assertion `left == right` failed: hardware: 260" in log and "hardware: 2 blocks matched" in log,log
            print("Lost carried state rejected on hardware.")
        else:
            assert trial.returncode==0 and all(f"{d}: 260 blocks matched" in log for d in ("hardware","warp")),log
            print(log)
            print(f"{len(cases)} fragments, {sum(len(s) for _,s,_ in cases)} symbols, {short} short and {long} long headers.")
