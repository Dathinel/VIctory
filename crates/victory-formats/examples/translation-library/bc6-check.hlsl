#include "bc6-wide.hlsli"
#include "bc6-add-endpoint.hlsli"

// One comparison per byte pair, 128 pairs per result word.
[numthreads(64,1,1)]
void main(uint3 id : SV_DispatchThreadID) {
    ByteAddressBuffer reference = ResourceDescriptorHeap[0];
    RWByteAddressBuffer results = ResourceDescriptorHeap[1];
    uint chunks = reference.Load(0);
    for (uint chunk = id.x; chunk < chunks; chunk += 64) {
        uint matches = 0;
        for (uint k = 0; k < 128; ++k) {
            uint pair = 128 * chunk + k;
            uint wide = bc6_wide(pair & 255, pair >> 8);
            uint combined = wide | (bc6_endpoint_residual(wide) << 16);
            uint offset = 4 + pair * 36;
            bool correct = combined == reference.Load(offset);
            uint predictor = (pair * 40503 + 97) & 65535;
            for (uint shift = 0; shift < 16; shift += 2) {
                uint residual = bc6_endpoint_residual(wide);
                uint low = bc6_add_endpoint(predictor, residual, shift);
                uint high = bc6_add_endpoint(predictor, residual, shift + 1);
                uint expected = reference.Load(offset + 4 + 2 * shift);
                correct = correct && ((low | (high << 16)) == expected);
            }
            matches += uint(correct);
        }
        results.Store(chunk * 4, matches);
    }
}
