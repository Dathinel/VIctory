"""Compare the HLSL fixture oracle to the actual arithmetic expressions in Flamingo."""
from fixture import synthetic_models
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

rustc, repo = (Path(s).resolve() for s in sys.argv[1:])
base = "5b2c0d07b08c5eab275d54e62a481276721991bc"
source = subprocess.check_output(["git", "-C", repo, "show", base + ":crates/victory-formats/src/bcpack.rs"], text=True)
bucket = source[source.index("fn bucket_of("):].split("\n}", 1)[0] + "\n}"
branch = source[source.index("            let freq = &t.freq"):source.index("            self.out = Some(sym as u8);")]
branch = branch.replace("self.model", "model").replace("self.x =", "let next =")
driver = """
use std::io::{self, Write};
struct Tables { start: [[u32;10];2], bucket: [[u8;256];2], freq: [[u32;9];2], recip: [[u64;9];2] }
fn main(){
 let mut t = Tables {start:[[65536;10];2],bucket:[[0;256];2],freq:[[0;9];2],recip:[[0;9];2]};
 for (m,row) in [[257,65279,0,0,0,0,0,0,0],[1,1,1,1,1,1,1,1,511]].iter().enumerate(){
  let mut at=0;
  for (b,&f) in row.iter().enumerate(){
   if f==0 {break}
   t.start[m][b]=at; t.freq[m][b]=f; t.recip[m][b]=(1u64<<32).div_ceil(u64::from(f)); at+=f<<b.saturating_sub(1);
  }
  assert_eq!(at,65536);
  for w in 0..256 {t.bucket[m][w]=t.start[m][1..].iter().filter(|&&v|v<=w as u32*256).count() as u8;}
 }
 let t=&t;
 let mut out=io::BufWriter::new(io::stdout().lock());
 for s in 0..65536u32 {for model in 0..2u8{
  let hi=if s==1 {256} else {257+s*40503%65279};
  let x=(hi<<16)|s;
  BODY
  out.write_all(&sym.to_le_bytes()).unwrap();out.write_all(&next.to_le_bytes()).unwrap();
 }}
}
"""
with TemporaryDirectory() as temp:
    tmp = Path(temp)
    (tmp / "oracle.rs").write_text(bucket + driver.replace("BODY", branch))
    subprocess.run([rustc, "-O", "oracle.rs"], cwd=tmp, check=True)
    result = subprocess.check_output([tmp / "oracle.exe"])
    _, expected = synthetic_models()
    assert result == expected and len(result) == 65536 * 16
    print("Flamingo", base, "reciprocal arithmetic and independently expanded symbols agree on 131072 symbol/state pairs.")
