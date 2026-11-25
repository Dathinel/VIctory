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
