//! Typed RTL path: every corpus design converts, and width/sign-sensitive expressions are
//! emitted at the width the source language evaluates them at. Behavioral agreement with a
//! reference simulator is checked separately by `scripts/rtl_cosim.sh`.

use std::path::PathBuf;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/rtl")
}

fn convert(src: &str) -> String {
    svcc::sv_rtl_to_sc("t.sv", src, &[])
        .unwrap_or_else(|e| panic!("conversion failed: {e}"))
        .0
}

#[test]
fn every_corpus_design_converts() {
    let mut n = 0;
    for entry in std::fs::read_dir(corpus_dir()).expect("testdata/rtl") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "sv") {
            let src = std::fs::read_to_string(&path).expect("read");
            let name = path.to_string_lossy().to_string();
            let (code, design) =
                svcc::sv_rtl_to_sc(&name, &src, &[]).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(code.contains("SC_MODULE("), "{name}: no module emitted");
            assert!(!design.tops.is_empty(), "{name}: no top");
            // A pure RTL header depends on SystemC only.
            assert!(!code.contains("<uvm>") && !code.contains("crave"), "{name}");
            n += 1;
        }
    }
    assert!(n >= 16, "expected the RTL corpus, found {n} files");
}

#[test]
fn carry_of_a_sum_survives_the_shift() {
    // (a + b) is 9 bits wide in a 9-bit context; the carry must reach bit 7 of the result.
    let code = convert(
        "module m(input logic [7:0] a, b, output logic [8:0] y);\n  assign y = (a + b) >> 1;\nendmodule\n",
    );
    assert!(
        code.contains("sc_dt::sc_uint<9>(a.read() + b.read()) >> 1"),
        "{code}"
    );
}

#[test]
fn eight_bit_context_truncates_before_the_shift() {
    // In an 8-bit context the sum wraps at 8 bits before it is shifted.
    let code = convert(
        "module m(input logic [7:0] a, b, output logic [7:0] y);\n  assign y = (a + b) >> 1;\nendmodule\n",
    );
    assert!(
        code.contains("sc_dt::sc_uint<8>(a.read() + b.read()) >> 1"),
        "{code}"
    );
}

#[test]
fn subtraction_wraps_before_comparison() {
    let code = convert(
        "module m(input logic [7:0] a, b, c, output logic y);\n  assign y = (a - b) < c;\nendmodule\n",
    );
    assert!(
        code.contains("sc_dt::sc_uint<8>(a.read() - b.read()) < c.read()"),
        "{code}"
    );
}

#[test]
fn blocking_read_back_uses_a_local_copy() {
    let code = convert(
        "module m(input logic [7:0] a, output logic [7:0] y);\n  always_comb begin\n    y = a;\n    y = y + 8'd1;\n  end\nendmodule\n",
    );
    assert!(
        code.contains("y_v = sc_dt::sc_uint<8>(y_v + 1U);"),
        "{code}"
    );
    assert!(code.contains("y.write(y_v);"), "{code}");
    // The block is not sensitive to the signal it assigns.
    assert!(code.contains("sensitive << a;"), "{code}");
}

#[test]
fn unsupported_constructs_fail_closed() {
    let err = svcc::sv_rtl_to_sc(
        "t.sv",
        "module m(input logic clk, inout wire [3:0] d);\nendmodule\n",
        &[],
    )
    .expect_err("inout must be rejected");
    assert!(err.message.contains("inout"), "{err}");
}
