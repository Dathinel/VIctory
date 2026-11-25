// Raw input: count, then 20 bytes per block (two endpoints, two padding
// bytes and sixteen unpacked ranks). Dispatch one group for this fixture.
[numthreads(1, 1, 1)]
void main() {
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    uint count = src.Load(0);
    for (uint block = 0; block < count; ++block) {
        uint base = 4 + 20 * block;
        uint2 packed = uint2((src.Load(base) & 0xffff) ^ 0x8080, 0);
        for (uint p = 0; p < 16; ++p) {
            uint ranks = src.Load(base + 4 + (p & ~3u));
            uint rank = (ranks >> (8 * (p & 3))) & 7;
            uint code = rank == 0 ? 0 : (rank == 7 ? 1 : rank + 1);
            uint shift = 16 + 3 * p;
            if (shift < 32) {
                packed.x |= code << shift;
                if (shift == 31) packed.y |= code >> 1;
            } else {
                packed.y |= code << (shift - 32);
            }
        }
        dst.Store2(8 * block, packed);
    }
}
