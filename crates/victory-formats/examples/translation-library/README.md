# Synthetic shader translations

This fork branch preserves source examples independently of pull request branches. BC5 has a [runnable example and test](../bc5/README.md).

| Source | Operation |
| --- | --- |
| bc1-records.hlsl | Reconstruct BC1 blocks from post-entropy records |
| bc4-alpha.hlsl | Pack two endpoints and sixteen ranks into a BC4 block |
| bc4-records.hlsl | Reconstruct BC4 blocks from post-entropy records |
| bc6-wide.hlsli, bc6-add-endpoint.hlsli | Interleave two byte lanes, decode the residual and add it to a reference endpoint |
| bc6-check.hlsl | Check both BC6H headers against reference words in a raw input buffer |

Compile each .hlsl file with DXC using shader model cs_6_6 and entry point main. The BC6H headers are included by bc6-check.hlsl. The examples specify their layouts in source. Use synthetic data. Reconstructors assume validated sizes and process one stream serially.

The synthetic-tests directory contains independent Rust fixture patches. Apply one at a time with git apply in a checkout of its base. The interleave, references and bc1-color tests use 940d32f69b22265690a0b5a15d6ded572ba33baf. The RGBA rounding test uses c4c775810712ddd7ebf082f37c42d426bb2fa369. The BC1 records fixture uses da82e444ec46a162ca779c82d4cf191a69182971. Compile bc1-records.hlsl to bc1-records.dxil in a temporary folder, set BC1_TEST_DIR to the folder, run the vic117_bc1_cpu test in victory-formats and then vic117_bc1_gpu in victory-shader. Use Windows and the host feature for GPU tests.

## BCPack entropy decoding

- [Look up a symbol bucket](bcpack-bucket/README.md)
- [Decode a symbol and update its state](bcpack-rans/README.md)
- [Read a contiguous synthetic fragment](bcpack-fragment/README.md)
- [Step one lane across a fragment boundary](bcpack-lane/README.md)
- [Schedule multiple lanes and recover entropy records](bcpack-schedule/README.md)
