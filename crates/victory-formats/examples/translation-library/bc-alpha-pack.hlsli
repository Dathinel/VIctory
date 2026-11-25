// Endpoints occupy the low two bytes; ranks_offset is word aligned.
uint2 bc_alpha_pack(ByteAddressBuffer src, uint endpoints, uint ranks_offset) {
    uint2 packed = uint2(endpoints ^ 0x8080, 0);
    for (uint p = 0; p < 16; ++p) {
        uint ranks = src.Load(ranks_offset + (p & ~3u));
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
    return packed;
}
