# BC4 blocks from an interleaved BCPack stream

The compute shader decodes one synthetic stream per dispatch. Twelve entropy lanes recover the record bytes, then the BC4 reconstructor applies dictionary or history references and packs two endpoints and sixteen indices into each eight-byte block. One thread performs both stages. This example has not been benchmarked.

Bind a word-padded raw input buffer at descriptor 0 and a raw writable buffer at descriptor 1. All integers are little endian. The input contains:

| Offset (bytes) | Contents |
| --- | --- |
| 0 | Block count (uint) |
| 4 | 64 rank bytes |
| 68 | 255 dictionary entries of eight bytes each, already packed as BC4 blocks |
| 2108 | Available compressed byte count (uint) |
| 2112 | 128 validated entropy-model rows, 332 bytes each, in the scheduler's layout |
| 44608 | 12 uint widths for BC4 (7, 7, 7, 7 followed by eight 5s) |
| 44656 | Interleaved compressed bytes |

For N blocks the UAV requires 56*N+8 bytes. Its first 8*N bytes hold the final BC4 blocks. The next two uints are status and compressed bytes consumed. The remainder is scratch, one uint per decoded record byte. Status 1 means success, 2 an empty destination, and 3 a lane decoding failure. The shader leaves the final block region untouched after an entropy error. Check the status before using that region. The host must validate extents, table contents, header widths, descriptor types and representability of address calculations before dispatch.

On Windows, use the test runner from the parent library README and a Flamingo checkout containing the pinned Rust commit:

    python verify.py PATH_TO_RUSTC PATH_TO_DXC PATH_TO_TEST_EXE FLAMINGO_CHECKOUT

The fixture uses generated tables with a nonzero dictionary and compiles Flamingo's public Rust decoder. Compressed tests check both final blocks and scratch records, bytes consumed, and the output guard. Complete inputs have seven unused trailing bytes inside the declared byte range. The half-length fixture sets the declared range to exactly the shortened stream; alignment padding is outside that range. The fixture also checks an empty destination, a half-length stream, and a negative control that swaps adjacent record bytes. The separate record-only entry point is exercised with 2 and 260 blocks.

### Testing the full 260-block stream

The original BC1 test runner reserves 2080 output bytes. To test a 260-block stream (14568 bytes including status and scratch), build the sized runner in a local checkout of Flamingo at commit 5b2c0d07b08c5eab275d54e62a481276721991bc. From the checkout root:

    git apply --check PATH_TO_EXAMPLE/gpu-runner.diff.txt
    git apply PATH_TO_EXAMPLE/gpu-runner.diff.txt
    cargo test --locked -p victory-shader --features host --lib --no-run

Use the resulting test executable as PATH_TO_TEST_EXE in the command above. The verifier detects the sized runner by its test name. It also accepts the original runner for the smaller cases. The sized runner checks eight guard bytes and reports the offset of the first difference. The large fixture exercises a history distance of 255 in block 259. A negative control changes just that history distance so the first mismatch must be inside that block.

The runner is an opt-in local test and requires its generated fixtures when selected. The positive comparisons run on hardware and WARP. Negative controls stop at the first hardware mismatch. The eight trailing bytes are checked for nonzero writes; this is not a general GPU bounds detector.
