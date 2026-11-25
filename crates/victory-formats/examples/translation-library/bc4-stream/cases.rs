#[test]
fn synthetic_bc4_streams() {
    let mut t = Tables::parse(&tables()).unwrap();
    for (i, entry) in t.bc4.iter_mut().enumerate() {
        entry.ep[0] = (3 * i + 1) as u8;
        entry.ep[1] = (5 * i + 2) as u8;
        for (p, rank) in entry.ix[..16].iter_mut().enumerate() {
            *rank = ((i + p) % 8) as u8;
        }
    }
    for n in [0usize, 2, 36, 260] {
        let mut records: Vec<Vec<u8>> = (0..n)
            .map(|i| {
                let mut r = vec![0, 1, (7 * i) as u8, (13 * i) as u8];
                r.extend((0..8).map(|k| ((5 * i + 7 * k) % 64) as u8));
                r
            })
            .collect();
        if n > 0 {
            records[0][1] = 255;
        }
        if n > 2 {
            records[2][0] = 1; // index from history, zero endpoint predictor
            records[3][0] = 2;
            records[3][1] = 0; // endpoint from history, zero index predictor
        }
        if n == 36 {
            records[35][1] = 35;
        }
        if n == 260 {
            records[120][1] = 77;
            records[259][1] = 255;
        }
        let stream_bytes = stream(&t, &BC4_LANES, 1, &records);
        let mut expected = vec![0; n * 8];
        let result = decode(4, &stream_bytes, &mut expected, &t);
        assert_eq!(result.is_ok(), n != 0);
        if n == 2 {
            // Derived independently from dictionary entry 254, pair ranks and signed residuals.
            assert_eq!(
                expected,
                [
                    0x7b, 0x78, 0x4f, 0x84, 0x2a, 0x1b, 0x5d, 0x98, 0x77, 0x71, 0x0c, 0xc7, 0x73,
                    0x3c, 0x47, 0x53
                ]
            );
        }
        let mut it = stream_bytes.iter().copied();
        let mut decoded = vec![0; n * 12];
        lanes(&t, &BC4_LANES, 1, n, |_| it.next(), &mut decoded).unwrap();
        assert_eq!(decoded, records.concat());
        assert_eq!(it.next(), None);

        let mut payload = (n as u32).to_le_bytes().to_vec();
        payload.extend(t.rank4);
        for entry in &t.bc4 {
            let mut packed = [0; 8];
            bc4_pack(entry, &mut packed);
            payload.extend(packed);
        }
        let mut compressed = payload.clone();
        let mut suffix = stream_bytes.clone();
        suffix.extend([0xa5; 7]);
        let mut with_suffix = vec![0; n * 8];
        assert_eq!(
            decode(4, &suffix, &mut with_suffix, &t).is_ok(),
            result.is_ok()
        );
        assert_eq!(with_suffix, expected);
        compressed.extend((suffix.len() as u32).to_le_bytes());
        for m in 0..128 {
            for v in t.start[m] {
                compressed.extend(v.to_le_bytes());
            }
            compressed.extend(t.bucket[m]);
            for v in t.freq[m] {
                compressed.extend(v.to_le_bytes());
            }
        }
        for v in BC4_LANES {
            compressed.extend(u32::from(v).to_le_bytes());
        }
        let payload_at = compressed.len();
        compressed.extend(&suffix);
        compressed.resize(compressed.len().next_multiple_of(4), 0);
        std::fs::write(format!("stream-{n}.bin"), &compressed).unwrap();
        let mut combined = expected.clone();
        combined.extend((if result.is_ok() { 1u32 } else { 2u32 }).to_le_bytes());
        combined.extend((stream_bytes.len() as u32).to_le_bytes());
        for &v in &decoded {
            combined.extend(u32::from(v).to_le_bytes());
        }
        std::fs::write(format!("combined-{n}.bin"), combined).unwrap();
        if n > 0 {
            let cut = stream_bytes.len() / 2;
            compressed.truncate(payload_at + cut);
            compressed[2108..2112].copy_from_slice(&(cut as u32).to_le_bytes());
            compressed.resize(compressed.len().next_multiple_of(4), 0);
            let mut out = vec![0; n * 8];
            assert!(decode(4, &stream_bytes[..cut], &mut out, &t).is_err());
            assert!(out.iter().all(|&v| v == 0));
            let mut partial = vec![0; n * 12];
            let mut bytes = stream_bytes[..cut].iter().copied();
            assert!(lanes(&t, &BC4_LANES, 1, n, |_| bytes.next(), &mut partial).is_err());
            out.extend(3u32.to_le_bytes());
            out.extend(((cut - bytes.len()) as u32).to_le_bytes());
            for v in partial {
                out.extend(u32::from(v).to_le_bytes());
            }
            std::fs::write(format!("short-stream-{n}.bin"), compressed).unwrap();
            std::fs::write(format!("short-combined-{n}.bin"), out).unwrap();
        }
        payload.extend(decoded);
        std::fs::write(format!("input-{n}.bin"), payload).unwrap();
        std::fs::write(format!("expected-{n}.bin"), expected).unwrap();
        eprintln!(
            "{n} BC4 blocks from {} compressed bytes",
            stream_bytes.len()
        );
    }
}
