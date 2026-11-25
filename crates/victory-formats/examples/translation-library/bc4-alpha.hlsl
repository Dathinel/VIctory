#include "bc-alpha-pack.hlsli"
// Raw input: count, then 20 bytes per block (two endpoints, two padding
// bytes and sixteen unpacked ranks). Dispatch one group for this fixture.
[numthreads(1, 1, 1)]
void main() {
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    uint count = src.Load(0);
    for (uint block = 0; block < count; ++block) {
        uint base = 4 + 20 * block;
        uint2 packed = bc_alpha_pack(src, src.Load(base) & 0xffff, base + 4);
        dst.Store2(8 * block, packed);
    }
}
