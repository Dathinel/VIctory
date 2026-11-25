# Decode BCPack into BC3 blocks

Dispatch one group with a validated raw input SRV at descriptor 0 and a raw writable buffer at descriptor 1. A single invocation schedules twelve entropy lanes, reconstructs endpoints and indices with independent back references, and writes packed BC3 blocks in order.

All multibyte fields are little endian. Input offsets are bytes.

| Offset | Contents |
| --- | --- |
| 0 | Block count N, uint |
| 4 | Available compressed bytes, uint |
| 8 | 64 bytes of alpha rank pairs |
| 72 | 256 bytes of color rank quads |
| 328 | 255 packed BC3 dictionary blocks, 16 bytes each |
| 4408 | 128 validated entropy rows of 332 bytes each |
| 46904 | Twelve uint lane widths: 3, 3, 7, 5, 5, 5, 5, 4, 5, 4, 7, 7 |
| 46952 | Interleaved compressed stream |

The UAV needs 112*N+8 bytes. Its first 16*N bytes hold BC3 blocks. Two uints follow: status (1 success, 2 empty destination, 3 entropy failure) and compressed bytes consumed. The remaining 96*N bytes hold one uint per decoded record byte. On entropy failure scratch can be partial and the shader skips reconstruction. The host must validate table values, all offsets and extents, final input word padding, header widths and 32-bit address arithmetic before dispatch.

From this directory on Windows, use the sized runner described in [the BC4 example](../bc4-stream/README.md):

    python verify.py PATH_TO_RUSTC PATH_TO_DXC PATH_TO_SIZED_TEST_EXE FLAMINGO_CHECKOUT

The test builds the pinned Rust decoder and its synthetic stream encoder. It checks full final blocks, recovered records, cursor, status and output guard on hardware and WARP. The dictionary and both rank tables are generated. Cases include independent endpoint/index predictors, history and dictionary reads, the zero predictor, a 255-block distance, an empty destination, an exact-length stream, one byte missing, a half-length stream and seven trailing bytes. The negative controls deliberately select the wrong endpoint predictor or transpose the color quads.
