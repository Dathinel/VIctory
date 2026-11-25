// Input SRV 0: header (block count, dictionary offset, records offset) in
// dwords, then the 256-byte rank table at byte 12. Header offsets are bytes.
// Dictionary entries are packed BC1 blocks. Each record contains 12 symbols.
// The caller supplies validated input extents and an 8-byte output per block.
uint load_byte(ByteAddressBuffer src, uint address) {
    return (src.Load(address & ~3u) >> (8 * (address & 3))) & 255;
}
uint zigzag(uint v) {
    return (v >> 1) ^ (0u - (v & 1));
}
uint2 predictor(ByteAddressBuffer src, RWByteAddressBuffer dst, uint dictionary,
                uint block, uint back) {
    if (back == 0) return uint2(0, 0);
    if (back <= block) return dst.Load2(8 * (block - back));
    return src.Load2(dictionary + 8 * (back - block - 1));
}
uint reconstruct_color(ByteAddressBuffer src, uint address, uint previous) {
    uint r = ((previous >> 11) + zigzag(load_byte(src, address))) & 31;
    uint g = ((previous >> 5) + zigzag(load_byte(src, address + 1))) & 63;
    uint b = (previous + zigzag(load_byte(src, address + 2))) & 31;
    return (r << 11) | (g << 5) | b;
}
[numthreads(1, 1, 1)]
void main() {
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    uint3 header = src.Load3(0);
    // One thread processes one stream in order because references can
    // address any of its previous 255 records. Dispatch exactly one group.
    for (uint i = 0; i < header.x; ++i) {
        uint record = header.z + 12 * i;
        uint ix_back = load_byte(src, record + 1);
        uint ep_back = (ix_back + zigzag(load_byte(src, record))) & 255;
        uint ep = predictor(src, dst, header.y, i, ep_back).x;
        uint ix = predictor(src, dst, header.y, i, ix_back).y;
        uint c0 = reconstruct_color(src, record + 2, ep);
        uint c1 = reconstruct_color(src, record + 5, ep >> 16);
        uint codes = 0;
        [unroll] for (uint q = 0; q < 4; ++q) {
            uint quad = load_byte(src, 12 + load_byte(src, record + 8 + q));
            [unroll] for (uint k = 0; k < 4; ++k) {
                uint pixel = 8 * (q >> 1) + 2 * (q & 1) + 4 * (k >> 1) + (k & 1);
                uint old_code = (ix >> (2 * pixel)) & 3;
                uint old_rank = ((old_code & 1) << 1) | ((old_code ^ (old_code >> 1)) & 1);
                uint rank = (old_rank + (quad >> (2 * k))) & 3;
                uint code = (((rank ^ (rank >> 1)) & 1) << 1) | (rank >> 1);
                codes |= code << (2 * pixel);
            }
        }
        dst.Store2(8 * i, uint2(c0 | (c1 << 16), codes));
        DeviceMemoryBarrier();
    }
}
