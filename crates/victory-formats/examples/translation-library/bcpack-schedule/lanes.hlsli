#include "../bcpack-lane/lane-step.hlsli"

// One invocation owns the stream. Widths and output symbols occupy one uint
// each. The input stream is byte packed and its containing SRV is word padded.
// The host validates sizes, 1..16 lanes, per > 0, model headers and offsets.
bool bcpack_lanes(ByteAddressBuffer src, RWByteAddressBuffer dst, uint rows,
                  uint widths, uint count, uint per, uint blocks, inout uint cursor,
                  uint end, uint output_base) {
    BcpackLane lanes[16];
    bool stalled[16];
    for (uint j = 0; j < count; ++j) {
        BcpackLane lane = (BcpackLane)0;
        lane.width = src.Load(widths + 4 * j);
        lane.need = 1;
        lane.latch = lane.symbol = 256;
        lanes[j] = lane;
        stalled[j] = false;
    }
    uint done = 0, pos = 0, idle = 0;
    while (pos < blocks * per) {
        uint taken = 0;
        bool moved = false;
        for (uint j = 0; j < count; ++j) {
            BcpackLane lane = lanes[j];
            uint bit = 1u << j;
            uint before = taken;
            if (!stalled[j]) bcpack_lane_step(src, rows, lane);
            bool waiting = lane.symbol != 256 && (done & bit) != 0;
            if (lane.need != 0 && !waiting && taken < 8 && cursor < end) {
                uint word = src.Load(cursor & ~3u);
                lane.latch = (word >> ((cursor & 3) * 8)) & 255;
                ++cursor;
                ++taken;
            }
            stalled[j] = (lane.need != 0 && before >= 8) || waiting;
            if (!stalled[j] && lane.symbol != 256) {
                uint index = (pos / per * count + j) * per + pos % per;
                dst.Store(output_base + 4 * index, lane.symbol);
                lane.symbol = 256;
                done |= bit;
                moved = true;
            }
            lanes[j] = lane;
        }
        if (done == ((1u << count) - 1)) {
            ++pos;
            done = 0;
            moved = true;
        }
        idle = taken != 0 || moved ? 0 : idle + 1;
        if (idle > 3) return false;
    }
    return true;
}
