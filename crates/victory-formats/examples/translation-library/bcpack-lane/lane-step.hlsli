#include "../bcpack-rans/decode-symbol.hlsli"

// Missing bytes and symbols are encoded as 256. Each model occupies 332 bytes:
// 10 uint starts, 256 byte hints, then 9 uint frequencies. Headers are validated
// by the caller (width >= header >> 6). The scheduler owns stalls and delivery.
struct BcpackLane {
    uint width, phase, long_header, reset, model, need, latch, symbol, x, left;
};

void bcpack_lane_step(ByteAddressBuffer tables, uint rows, inout BcpackLane lane) {
    if (lane.phase != 3) {
        if (lane.latch == 256) return;
        uint b = lane.latch;
        lane.latch = 256;
        lane.need = 1;
        if (lane.phase == 0) {
            lane.model = (lane.width - (b >> 6)) * 16 + (b & 15);
            lane.reset = (b >> 4) & 1;
            lane.long_header = (b >> 5) & 1;
            lane.left = 0;
            lane.phase = 1;
        } else if (lane.phase == 1 && lane.long_header != 0) {
            lane.left = b;
            lane.phase = 2;
        } else {
            lane.left = lane.phase == 1 ? b + 1 : ((b << 8) | lane.left) + 257;
            lane.phase = 3;
        }
        return;
    }

    uint x = lane.x;
    if (x < (1u << 24) && lane.latch != 256) {
        --lane.left;
        x = (x << 8) | lane.latch;
        lane.latch = 256;
    }
    lane.need = uint(lane.left != 0 && lane.latch == 256);
    if (x > (1u << 24)) {
        uint row = rows + 332 * lane.model;
        uint2 decoded = bcpack_decode_symbol(tables, row, row + 40, row + 296, x);
        lane.x = decoded.y;
        lane.symbol = decoded.x;
    } else if (lane.left != 0) {
        lane.x = x;
    } else {
        lane.phase = 0;
        lane.need = uint(lane.latch == 256);
        if (lane.reset != 0) lane.x = 0;
    }
}
