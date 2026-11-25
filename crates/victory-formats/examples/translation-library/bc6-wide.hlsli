// Expand an 8-bit lane onto bits 0, 2, ..., 14.
uint bc6_spread8(uint lane) {
    lane &= 255;
    lane = (lane | (lane << 4)) & 0x0f0f;
    lane = (lane | (lane << 2)) & 0x3333;
    return (lane | (lane << 1)) & 0x5555;
}

uint bc6_wide(uint even_lane, uint odd_lane) {
    return bc6_spread8(even_lane) | (bc6_spread8(odd_lane) << 1);
}

uint bc6_endpoint_residual(uint wide) {
    return ((wide >> 1) ^ (0u - (wide & 1))) & 0xffff;
}
