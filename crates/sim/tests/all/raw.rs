//! Raw files written by ngspice 47 (fixtures, so this runs without ngspice).

use sim::raw;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn binary_transient() {
    let plots = raw::parse(&fixture("tran.raw")).unwrap();
    assert_eq!(plots.len(), 1);
    let p = &plots[0];
    assert_eq!(p.name, "Transient Analysis");
    assert!(!p.complex);
    let names: Vec<_> = p.vectors.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["time", "v(in)", "v(out)", "i(v1)"]);
    assert_eq!(p.len(), 126);
    let time = &p.vector("time").unwrap().re;
    assert_eq!(time[0], 0.0);
    assert!((time[p.len() - 1] - 1e-3).abs() < 1e-12);
    assert!(time.windows(2).all(|w| w[1] > w[0]));
    // The pulse is 1 V after its 1 ns rise.
    assert!((p.vector("v(in)").unwrap().re[10] - 1.0).abs() < 1e-9);
}

#[test]
fn binary_ac_and_op_in_one_file() {
    let plots = raw::parse(&fixture("ac_op.raw")).unwrap();
    let names: Vec<_> = plots.iter().map(|p| p.name.as_str()).collect();
    // ngspice writes the last analysis first.
    assert_eq!(names, ["AC Analysis", "Operating Point"]);
    let ac = &plots[0];
    assert!(ac.complex);
    assert_eq!(ac.len(), 5);
    let f = &ac.vector("frequency").unwrap().re;
    assert_eq!(f[0], 1.0);
    assert!((f[4] - 100.0).abs() < 1e-9);
    let out = ac.vector("v(out)").unwrap();
    // RC = 1 ms: at 100 Hz, |H| = 1/sqrt(1 + (2π·0.1)²).
    let m = out.magnitude()[4];
    let expected = 1.0 / (1.0 + (2.0 * std::f64::consts::PI * 0.1f64).powi(2)).sqrt();
    assert!((m - expected).abs() < 1e-6, "{m} vs {expected}");
    let op = &plots[1];
    assert_eq!(op.len(), 1);
    assert_eq!(op.vectors[0].name, "v(in)");
    assert_eq!(op.vector("v(in)").unwrap().re, vec![1.0]);
}

#[test]
fn ascii_ac() {
    let ascii = raw::parse(&fixture("ac_ascii.raw")).unwrap();
    let binary = raw::parse(&fixture("ac_op.raw")).unwrap();
    assert_eq!(ascii.len(), 1);
    let (a, b) = (&ascii[0], &binary[0]);
    assert_eq!(a.len(), b.len());
    for name in ["frequency", "v(out)", "i(v1)"] {
        let (x, y) = (a.vector(name).unwrap(), b.vector(name).unwrap());
        for (p, q) in x.magnitude().iter().zip(y.magnitude()) {
            assert!((p - q).abs() <= 1e-9 * q.abs().max(1e-12), "{name}: {p} vs {q}");
        }
    }
}

#[test]
fn truncated_data_keeps_whole_points() {
    let mut data = fixture("tran.raw");
    data.truncate(data.len() - 40);
    let plots = raw::parse(&data).unwrap();
    assert_eq!(plots[0].len(), 124);
}
