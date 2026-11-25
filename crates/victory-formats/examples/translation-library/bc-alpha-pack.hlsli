// Endpoints occupy the low two bytes; four words hold sixteen byte ranks.
uint2 bc_alpha_pack_words(uint endpoints, uint4 rank_words) {
    uint2 packed = uint2(endpoints ^ 0x8080, 0);
    for (uint p = 0; p < 16; ++p) {
        uint ranks = rank_words[p / 4];
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

uint2 bc_alpha_pack(ByteAddressBuffer src, uint endpoints, uint ranks_offset) {
    return bc_alpha_pack_words(endpoints, src.Load4(ranks_offset));
}
