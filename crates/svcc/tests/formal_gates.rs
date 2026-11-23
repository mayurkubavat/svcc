//! The formal gates of the typed RTL path: what they prove, and that they fail when the
//! generated C++ (or a golden model) does not compute what the SystemVerilog computes.

use svcc::prove::{prove_rtl, prove_sec, RtlProveOptions, SecOptions};
use svcc_equiv::pair::PairVerdict;
use svcc_front_sc::hdl_lift::lift_hdl_design;

fn prove(sv: &str) -> svcc::prove::RtlProveReport {
    prove_rtl("t.sv", sv, &RtlProveOptions::default()).expect("prove_rtl")
}

/// Proves `sv` against a header derived from the generated one by `edit`.
fn prove_edited(sv: &str, edit: impl Fn(&str) -> String) -> svcc::prove::RtlProveReport {
    let (code, _) = svcc::sv_rtl_to_sc("t.sv", sv, &[]).expect("rtl2sc");
    let edited = edit(&code);
    assert_ne!(code, edited, "the edit did not change the header:\n{code}");
    let opts = RtlProveOptions {
        header: Some(edited),
        ..Default::default()
    };
    prove_rtl("t.sv", sv, &opts).expect("prove_rtl")
}

fn cpp_verdict(r: &svcc::prove::RtlProveReport) -> PairVerdict {
    match &r.generated_cpp {
        Some(p) => p.verdict,
        None => panic!("generated C++ not checked: {:?}", r.cpp_unchecked),
    }
}

const AVG: &str = "module avg(input logic [7:0] a, b, output logic [7:0] y);\n  assign y = (a + b) >> 1;\nendmodule\n";
const WRAP: &str = "module wrap(input logic [7:0] a, b, c, output logic lt);\n  assign lt = (a - b) < c;\nendmodule\n";

#[test]
fn generated_systemc_is_proved_against_its_source() {
    let r = prove(include_str!("../../../testdata/sv/alu_pipe_rtl.sv"));
    assert_eq!(r.lowering.verdict, PairVerdict::ProvedInductive);
    assert_eq!(cpp_verdict(&r), PairVerdict::ProvedInductive);
    // The pipeline registers are paired by name, so the proof is by induction, not bounded.
    assert!(r.generated_cpp.unwrap().states_paired >= 3);
    for sv in [AVG, WRAP] {
        assert_eq!(cpp_verdict(&prove(sv)), PairVerdict::ProvedInductive);
    }
}

/// The first bug the old round-trip check missed: `(a + b) >> 1` on 8-bit operands, with
/// the C++ shifting the 64-bit sum instead of the 8-bit one.
#[test]
fn sum_shifted_at_64_bits_is_caught() {
    let r = prove_edited(AVG, |c| {
        c.replace(
            "sc_dt::sc_uint<8>(a.read() + b.read()) >> 1U",
            "(a.read() + b.read()) >> 1U",
        )
    });
    // The lowering is still right; it is the C++ that is wrong.
    assert_eq!(r.lowering.verdict, PairVerdict::ProvedInductive);
    let cpp = r.generated_cpp.expect("checked");
    assert_eq!(cpp.verdict, PairVerdict::Mismatch);
    assert!(
        cpp.notes.iter().any(|n| n.contains("spec y")),
        "{:?}",
        cpp.notes
    );
}

/// The second: `(a - b) < c`, with the C++ comparing the 64-bit difference.
#[test]
fn difference_compared_at_64_bits_is_caught() {
    let r = prove_edited(WRAP, |c| {
        c.replace(
            "sc_dt::sc_uint<8>(a.read() - b.read()) < c.read()",
            "(a.read() - b.read()) < c.read()",
        )
    });
    assert_eq!(cpp_verdict(&r), PairVerdict::Mismatch);
}

#[test]
fn a_dropped_sign_conversion_is_caught() {
    let sv = "module m(input logic signed [7:0] a, input logic [7:0] b, output logic lt);\n  assign lt = a < $signed(b);\nendmodule\n";
    assert_eq!(cpp_verdict(&prove(sv)), PairVerdict::ProvedInductive);
    let r = prove_edited(sv, |c| c.replace("sc_dt::sc_int<8>(b.read())", "b.read()"));
    assert_eq!(cpp_verdict(&r), PairVerdict::Mismatch);
}

#[test]
fn an_unguarded_division_is_undefined_behaviour() {
    let sv =
        "module d(input logic [7:0] a, b, output logic [7:0] q);\n  assign q = a / b;\nendmodule\n";
    assert_eq!(cpp_verdict(&prove(sv)), PairVerdict::ProvedInductive);
    let r = prove_edited(sv, |c| {
        c.replace(
            "b.read() != 0 ? a.read() / b.read() : 0U",
            "a.read() / b.read()",
        )
    });
    let cpp = r.generated_cpp.expect("checked");
    assert_eq!(cpp.verdict, PairVerdict::Mismatch);
    assert!(
        cpp.notes.iter().any(|n| n.contains("undefined behaviour")),
        "{:?}",
        cpp.notes
    );
}

#[test]
fn a_missing_sensitivity_entry_is_rejected() {
    let (code, _) = svcc::sv_rtl_to_sc("t.sv", AVG, &[]).expect("rtl2sc");
    let edited = code.replace("sensitive << a << b;", "sensitive << a;");
    assert_ne!(code, edited);
    let e = lift_hdl_design(&edited).expect_err("must be rejected");
    assert!(e.message.contains("not sensitive to 'b'"), "{e}");
}

#[test]
fn cpp_outside_the_subset_is_rejected_not_guessed() {
    let bad = r#"
SC_MODULE(bad_heap_mod) {
  sc_core::sc_in<bool> clk{"clk"};
  void proc() {
    int* p = new int(42);
    delete p;
  }
  SC_CTOR(bad_heap_mod) { SC_METHOD(proc); }
};
"#;
    assert!(lift_hdl_design(bad).is_err());
}

#[test]
fn c2rtl_golden_model_against_pipelined_rtl() {
    let golden = include_str!("../../../testdata/sc/alu_golden_c2rtl.hpp");
    let rtl = include_str!("../../../testdata/sv/alu_pipe_rtl.sv");
    let opts = SecOptions {
        latency: 2,
        ..Default::default()
    };
    let ok = prove_sec("alu_golden.hpp", golden, "alu_pipe_rtl.sv", rtl, &opts).expect("sec");
    // Holds from an arbitrary state of the pipeline, so for every run.
    assert_eq!(ok.verdict, PairVerdict::ProvedInductive);

    let buggy = rtl.replace("a_s1 ^ b_s1", "a_s1 | b_s1");
    let bad = prove_sec("alu_golden.hpp", golden, "alu_buggy.sv", &buggy, &opts).expect("sec");
    assert_eq!(bad.verdict, PairVerdict::Mismatch);

    let wrong_latency = SecOptions {
        latency: 1,
        ..Default::default()
    };
    let off = prove_sec(
        "alu_golden.hpp",
        golden,
        "alu_pipe_rtl.sv",
        rtl,
        &wrong_latency,
    )
    .expect("sec");
    assert_eq!(off.verdict, PairVerdict::Mismatch);
}

#[test]
fn c2rtl_bounded_loop_and_compound_assignment() {
    let golden = r#"
// svcc:c2rtl
bool xor_parity4(sc_dt::sc_uint<4> x) {
  bool p = 0;
  for (int i = 0; i < 4; ++i) {
    p ^= ((x >> i) & 1U);
  }
  return p;
}
"#;
    let rtl = "module xor_parity4_rtl(input logic [3:0] x, output logic result);\n  assign result = x[0] ^ x[1] ^ x[2] ^ x[3];\nendmodule\n";
    let r = prove_sec(
        "parity.hpp",
        golden,
        "parity.sv",
        rtl,
        &SecOptions::default(),
    )
    .expect("sec");
    assert_eq!(r.verdict, PairVerdict::ProvedInductive);
}

/// `uint16_t * uint16_t` is computed in `int` and overflows for large operands: undefined
/// behaviour that a check which reads the C++ as if it were SystemVerilog cannot see.
#[test]
fn c2rtl_promoted_multiplication_overflow_is_flagged() {
    let rtl = "module mac(input logic [15:0] a, b, output logic [31:0] result);\n  assign result = a * b;\nendmodule\n";
    let bad = "// svcc:c2rtl\nuint32_t mac(uint16_t a, uint16_t b) {\n  uint32_t prod = a * b;\n  return prod;\n}\n";
    let r = prove_sec("mac.hpp", bad, "mac.sv", rtl, &SecOptions::default()).expect("sec");
    assert_eq!(r.verdict, PairVerdict::Mismatch);
    assert!(
        r.notes.iter().any(|n| n.contains("undefined behaviour")),
        "{:?}",
        r.notes
    );

    let good = bad.replace("a * b", "uint32_t(a) * b");
    let r = prove_sec("mac.hpp", &good, "mac.sv", rtl, &SecOptions::default()).expect("sec");
    assert_eq!(r.verdict, PairVerdict::ProvedInductive);
}

#[test]
fn two_different_designs_are_not_equal() {
    let a = "module m(input logic clk, input logic [3:0] d, output logic [3:0] q);\n  always_ff @(posedge clk) q <= q + d;\nendmodule\n";
    let b = a.replace("q + d", "q - d");
    let same = prove_sec("a.sv", a, "a2.sv", a, &SecOptions::default()).expect("sec");
    assert_eq!(same.verdict, PairVerdict::ProvedInductive);
    let diff = prove_sec("a.sv", a, "b.sv", &b, &SecOptions::default()).expect("sec");
    assert_eq!(diff.verdict, PairVerdict::Mismatch);
}

#[test]
fn array_ports_are_proved_element_by_element() {
    let sv = "module aport(input logic clk, input logic [7:0] d [0:3], input logic [1:0] sel,\n  output logic [7:0] q [0:3], output logic [7:0] y);\n  assign y = d[sel];\n  always_ff @(posedge clk) for (int i = 0; i < 4; i++) q[i] <= d[i] + 8'd1;\nendmodule\n";
    let r = prove(sv);
    let cpp = r.generated_cpp.expect("checked");
    assert_eq!(cpp.verdict, PairVerdict::ProvedInductive);
    assert_eq!(cpp.outputs_compared, 5);
    let bad = prove_edited(sv, |c| c.replace("+ 1U", "+ 2U"));
    assert_eq!(cpp_verdict(&bad), PairVerdict::Mismatch);
}

/// A form the reader does not model (here the lambda the emitter writes for a long
/// replication into a wide vector) is reported as not checked, never as proved.
#[test]
fn an_unmodelled_form_is_not_covered_rather_than_passed() {
    let sv = "module repl(input logic [3:0] a, output logic [127:0] y);\n  assign y = {32{a}};\nendmodule\n";
    let r = prove(sv);
    assert_eq!(r.lowering.verdict, PairVerdict::ProvedInductive);
    assert!(r.generated_cpp.is_none());
    assert!(r.cpp_unchecked.is_some());
    assert!(!r.cpp_ok());
}

#[test]
fn a_design_on_two_clocks_is_outside_the_proof_model() {
    let sv = "module two(input logic c1, c2, input logic d, output logic q1, q2);\n  always_ff @(posedge c1) q1 <= d;\n  always_ff @(posedge c2) q2 <= d;\nendmodule\n";
    let items =
        svcc::prove::prove_rtl_file("two.sv", sv, &RtlProveOptions::default()).expect("run");
    assert!(matches!(
        items.as_slice(),
        [svcc::prove::RtlFileItem::NotCovered { .. }]
    ));
}

const COUNTER: &str = "module cnt (input logic clk, input logic rst, output logic [7:0] q);\n  always_ff @(posedge clk) if (rst) q <= 8'd0; else q <= q + 8'd1;\nendmodule\n";

#[test]
fn a_difference_beyond_the_bound_is_only_bounded_not_proved() {
    // The edited header wraps at 200, which 8 steps from reset never reach: induction finds
    // the difference, the bounded check does not. The CLI exits 2 for this verdict.
    let r = prove_edited(COUNTER, |h| {
        h.replace(
            "q.write(sc_dt::sc_uint<8>(q.read() + 1U));",
            "q.write(q.read() == 200U ? sc_dt::sc_uint<8>(0U) : sc_dt::sc_uint<8>(q.read() + 1U));",
        )
    });
    assert_eq!(cpp_verdict(&r), PairVerdict::BoundedNoDifference);
}

#[test]
fn a_multi_file_design_with_a_parameter_override_is_proved() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/rtl_multi");
    let args: Vec<String> = vec![
        format!("{dir}/top.sv"),
        format!("{dir}/acc.sv"),
        "-G".into(),
        "W=12".into(),
    ];
    let opts = RtlProveOptions {
        top: Some("top".into()),
        ..Default::default()
    };
    let r = svcc::prove::prove_rtl_args(&args, &opts).expect("prove_rtl_args");
    assert_eq!(r.lowering.verdict, PairVerdict::ProvedInductive);
    assert_eq!(cpp_verdict(&r), PairVerdict::ProvedInductive);
    assert!(r.generated_cpp.unwrap().states_paired >= 2);
}
