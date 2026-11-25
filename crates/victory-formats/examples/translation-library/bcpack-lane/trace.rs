use std::io::{self, Write};

struct Tables {
    freq: [[u32; 9]; 128],
    start: [[u32; 10]; 128],
    bucket: [[u8; 256]; 128],
    recip: [[u64; 9]; 128],
}
type Error = ();

fn words(w: &mut impl Write, values: &[u32]) {
    for v in values {
        w.write_all(&v.to_le_bytes()).unwrap();
    }
}
fn snapshot(w: &mut impl Write, l: &Lane) {
    words(
        w,
        &[
            l.width.into(),
            l.phase.into(),
            l.long as u32,
            l.reset as u32,
            l.model.into(),
            l.need as u32,
            l.latch.map(u32::from).unwrap_or(256),
            l.out.map(u32::from).unwrap_or(256),
            l.x,
            l.left,
        ],
    );
}
fn main() {
    let mut w = io::BufWriter::new(io::stdout().lock());
    let mut t = Tables {
        freq: [[0; 9]; 128],
        start: [[65536; 10]; 128],
        bucket: [[0; 256]; 128],
        recip: [[0; 9]; 128],
    };
    for m in 0..128 {
        let g = m / 16;
        let mut at = 0;
        for b in 0..g + 2 {
            let f = (1u32 << (15 - g)) + if b == 0 { m as u32 % 16 } else { 0 }
                - if b == 1 { m as u32 % 16 } else { 0 };
            t.freq[m][b] = f;
            t.recip[m][b] = (1u64 << 32).div_ceil(u64::from(f));
            t.start[m][b] = at;
            at += f << b.saturating_sub(1);
        }
        assert_eq!(at, 65536);
        for j in 0..256 {
            t.bucket[m][j] = t.start[m][1..]
                .iter()
                .filter(|&&v| v <= j as u32 * 256)
                .count() as u8;
        }
        words(&mut w, &t.start[m]);
        w.write_all(&t.bucket[m]).unwrap();
        words(&mut w, &t.freq[m]);
    }
    let mut ends = [0; 2];
    let (mut outputs, mut pending, mut long_headers, mut parked) = (0, 0, 0, 0);
    for i in 0..260u32 {
        let targeted = i % 4 < 2;
        let mut l = Lane {
            width: (3 + i / 4 % 5) as u8,
            phase: if targeted { 3 } else { 0 },
            need: !targeted,
            reset: i % 2 == 1,
            x: if targeted { LOW >> 8 } else { 0 },
            left: if targeted { 1 } else { 0 },
            model: (i % 128) as u8,
            latch: if targeted { Some(0) } else { None },
            ..Lane::default()
        };
        if i % 4 == 2 {
            l.phase = 3;
            l.left = 4;
            l.need = false;
            l.latch = Some((i * 13) as u8);
            l.x = if i % 12 == 2 {
                LOW
            } else {
                0xf0010000 | (i * 211 & 65535)
            };
        }
        if i % 16 == 3 {
            l.phase = 3;
            l.x = LOW;
            l.left = 0;
            l.need = false;
            l.latch = Some(0);
        }
        snapshot(&mut w, &l);
        for step in 0..8u32 {
            let mut feed = 256;
            if l.need && l.latch.is_none() && (step + i) % 5 != 4 {
                feed = if l.phase == 0 {
                    (i * 71 + step * 11) & 255
                } else {
                    (step * 73 + i * 41) & 255
                };
                l.latch = Some(feed as u8);
            }
            words(&mut w, &[feed]);
            l.step(&t).unwrap();
            if targeted && step == 0 {
                assert_eq!(l.phase, 0);
                assert_eq!(l.x, if l.reset { 0 } else { LOW >> 8 });
                ends[l.reset as usize] += 1;
            }
            if i % 16 == 3 && step == 0 {
                assert_eq!((l.phase, l.need, l.latch), (0, false, Some(0)));
            }
            outputs += usize::from(l.out.is_some());
            pending += usize::from(l.out.is_some() && l.latch.is_some());
            long_headers += usize::from(l.phase == 2);
            parked += usize::from(l.phase == 3 && l.x == LOW && l.left != 0 && l.latch.is_some());
            snapshot(&mut w, &l);
            l.out = None;
        }
    }
    assert!(outputs > 0 && pending > 0 && long_headers > 0 && parked > 0);
    eprintln!(
        "Emitted {outputs}, emitted with pending byte {pending}, long length phase {long_headers}, parked with pending byte {parked}"
    );
    eprintln!(
        "Terminal-byte transitions checked: retained {}, reset {}",
        ends[0], ends[1]
    );
}
