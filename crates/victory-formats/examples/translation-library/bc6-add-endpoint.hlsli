// bc6-add-endpoint.hlsli
// VIC-124: HLSL equivalent of the Rust u16 expression in bc6:
//   r.wrapping_add(res << shift) & (0xffffu16 << shift)

#ifndef BC6_ADD_ENDPOINT_HLSLI
#define BC6_ADD_ENDPOINT_HLSLI

// reference, residual: u16 values widened to uint (upper 16 bits are ignored).
// shift: 0..15.
// Returns the u16 result widened to uint (upper 16 bits always zero).
//
// Signed residuals arrive as two's-complement u16 (e.g. -1 == 0xFFFF), so
// addition mod 2^16 is already correct. The final mask drops the carry out of
// bit 15, any residual bits shifted past bit 15, and the unused low `shift` bits.
uint bc6_add_endpoint(uint reference, uint residual, uint shift)
{
    uint s    = shift & 15u;
    uint mask = (0xFFFFu << s) & 0xFFFFu;
    return (reference + (residual << s)) & mask;
}

#endif // BC6_ADD_ENDPOINT_HLSLI
