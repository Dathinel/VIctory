use std::fs;

fn main() {
    for count in [0u32, 1, 64, 65, 449] {
        let mut input = count.to_le_bytes().to_vec();
        let mut alpha_input = input.clone();
        let mut expected = Vec::new();
        let mut alpha_expected = Vec::new();
        for i in 0..count {
            let mut v = Plain::default();
            for (j, e) in v.ep.iter_mut().enumerate() {
                let raw = (i * 29 + j as u32 * 47) as u8;
                *e = raw & [31, 63, 31, 31, 63, 31, 255, 255][j];
            }
            for (j, k) in v.ix.iter_mut().enumerate() {
                *k = (i * 37 + j as u32 * 17 + (j as u32 / 4) * 3) as u8;
            }
            if i == 0 {
                // Red and blue endpoints; alpha endpoints straddle the sign flip.
                v.ep = [31, 0, 0, 0, 0, 31, 127, 128];
                v.ix.fill(0);
                v.ix[5] = 1;
                v.ix[16] = 1;
            }
            if i >= 257 {
                // Isolate one position and channel; the upper rank bits are ignored.
                v.ix.fill(0xf8);
                let case = (i - 257) as usize;
                if case < 128 {
                    v.ix[case / 8] |= (case % 8) as u8;
                } else {
                    let color = case - 128;
                    v.ix[16 + color / 4] |= (color % 4) as u8;
                }
            }
            input.extend(v.ep);
            input.extend(v.ix);
            alpha_input.extend([v.ep[6], v.ep[7], 0xa5, 0x5a]);
            alpha_input.extend(&v.ix[..16]);
            let mut block = [0u8; 16];
            bc3_pack(&v, &mut block);
            if i == 0 {
                assert_eq!(block, [255, 0, 0, 0, 1, 0, 0, 0, 0, 248, 31, 0, 2, 0, 0, 0]);
            }
            expected.extend(block);
            alpha_expected.extend(&block[..8]);
        }
        for (stem, data) in [
            ("input", input),
            ("expected", expected),
            ("alpha-input", alpha_input),
            ("alpha-expected", alpha_expected),
        ] {
            fs::write(format!("{stem}-{count}.bin"), data).unwrap();
        }
    }
}
