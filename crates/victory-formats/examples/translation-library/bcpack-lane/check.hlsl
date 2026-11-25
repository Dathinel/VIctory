#include "lane-step.hlsli"

[numthreads(64,1,1)]
void main(uint3 id : SV_DispatchThreadID) {
    if (id.x >= 64 || id.y != 0 || id.z != 0) return;
    ByteAddressBuffer src = ResourceDescriptorHeap[0];
    RWByteAddressBuffer dst = ResourceDescriptorHeap[1];
    for (uint i=id.x; i<src.Load(0); i+=64) {
        uint at = 4 + 332*128 + i*(40+8*44);
        uint4 a=src.Load4(at), b=src.Load4(at+16);
        uint2 c=src.Load2(at+32);
        BcpackLane lane = {a.x,a.y,a.z,a.w,b.x,b.y,b.z,b.w,c.x,c.y};
        uint passed=0;
        for (uint step=0; step<8; ++step) {
            at += step==0 ? 40 : 44;
            uint feed=src.Load(at);
            if (feed!=256) lane.latch=feed;
            bcpack_lane_step(src,4,lane);
            bool ok=all(src.Load4(at+4)==uint4(lane.width,lane.phase,lane.long_header,lane.reset))
                 && all(src.Load4(at+20)==uint4(lane.model,lane.need,lane.latch,lane.symbol))
                 && all(src.Load2(at+36)==uint2(lane.x,lane.left));
            passed+=uint(ok);
            lane.symbol=256;
        }
        dst.Store2(i*8,uint2(passed,0));
    }
}
