//! Generates a realistic-looking FCS 3.0 file with 4 parameters and 4000 events
//! containing two Gaussian clusters, for end-to-end CLI testing of `flowers`.


fn ptr(n: usize) -> [u8; 8] {
    let s = format!("{n:>8}");
    let mut b = [0u8; 8];
    b.copy_from_slice(s.as_bytes());
    b
}

fn main() {
    let event_count: usize = 4000;
    let param_count: usize = 4;
    let mut text = Vec::new();
    text.push(b'\\');
    text.extend_from_slice(format!("$TOT\\{event_count}\\").as_bytes());
    text.extend_from_slice(format!("$PAR\\{param_count}\\").as_bytes());
    let names = ["FSC-H", "SSC-H", "FL1-H", "FL2-H"];
    let descs = [
        "Forward Scatter-Height",
        "Side Scatter-Height",
        "Fluorescence 1-Height",
        "Fluorescence 2-Height",
    ];
    for (n, (name, desc)) in names.iter().zip(descs.iter()).enumerate() {
        let n = n + 1;
        text.extend_from_slice(format!("$P{n}N\\{name}\\").as_bytes());
        text.extend_from_slice(format!("$P{n}S\\{desc}\\").as_bytes());
        text.extend_from_slice(format!("$P{n}E\\0\\").as_bytes());
        text.extend_from_slice(format!("$P{n}R\\4.0\\").as_bytes());
    }
    text.extend_from_slice(b"$BYTEORD\\1,2,3,4\\");
    text.extend_from_slice(b"$MODE\\L\\");

    let text_start = 59usize;
    let text_end = text_start + text.len();
    let data_start = text_end;

    let mut buf = Vec::new();
    buf.extend_from_slice(b"FCS3.0    ");
    buf.extend_from_slice(&ptr(text_start));
    buf.extend_from_slice(&ptr(text_end));
    buf.extend_from_slice(&ptr(data_start));
    buf.extend_from_slice(&ptr(data_start)); // data end (unused)
    buf.extend_from_slice(&ptr(data_start + 1)); // analysis start
    buf.extend_from_slice(&ptr(data_start + 2)); // analysis end
    buf.extend_from_slice(&text);

    // Two clusters: cluster A low on all channels; cluster B high on FL1/FL2.
    let mut rng = 42u64;
    for _ in 0..event_count {
        let cluster_b = (rng % 2) == 0;
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        for _j in 0..param_count {
            let base = if cluster_b { 2.5 } else { 0.5 };
            let noise = ((rng % 100) as f32 / 100.0 - 0.5) * 0.4;
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let val = base + noise;
            buf.extend_from_slice(&val.to_le_bytes());
        }
    }

    std::fs::write("sample.fcs", &buf).expect("write sample.fcs");
    println!(
        "wrote sample.fcs: {} events x {} params ({} bytes)",
        event_count,
        param_count,
        buf.len()
    );
}
