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
            // Each corpus file's top module is named after the file, as scripts/rtl_cosim.sh
            // assumes; a module that instantiates itself is never picked as a top on its own.
            let top = path
                .file_stem()
                .expect("stem")
                .to_string_lossy()
                .to_string();
            let (code, design) =
                svcc::sv_rtl_to_sc(&name, &src, &[top]).unwrap_or_else(|e| panic!("{name}: {e}"));
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

#[test]
fn wildcard_connection_binds_every_port() {
    let code = convert(
        "module leaf(input logic a, input logic b, output logic y);\n  assign y = a & b;\nendmodule\nmodule m(input logic a, input logic b, output logic y);\n  leaf u (.*);\nendmodule\n",
    );
    for port in ["u.a(a);", "u.b(b);", "u.y(y);"] {
        assert!(code.contains(port), "missing {port}:\n{code}");
    }
}

#[test]
fn undriven_output_is_an_error() {
    let err = svcc::sv_rtl_to_sc(
        "t.sv",
        "module m(input logic a, output logic y, output logic z);\n  assign y = a;\nendmodule\n",
        &[],
    )
    .expect_err("an output without a driver must be rejected");
    assert!(err.message.contains("'z' is never driven"), "{err}");
}

#[test]
fn non_ansi_output_redeclared_with_initializer_is_driven() {
    // `output c; wire c = ~a;` declares one port whose net carries a continuous assignment.
    // Quartus megafunction wrappers are written this way; the driver used to be dropped.
    let code = convert("module m(a, c);\n  input a;\n  output c;\n  wire c = ~a;\nendmodule\n");
    assert!(code.contains("c.write("), "port 'c' has no driver:\n{code}");
}

#[test]
fn assignment_list_targets_are_independent() {
    // `assign y = t, t = ~a;`: y must follow t, not t's value before this assignment ran.
    let code = convert("module m(input logic [3:0] a, output logic [3:0] y);\n  logic [3:0] t;\n  assign y = t, t = ~a;\nendmodule\n");
    assert!(code.contains("void assign_y()") && code.contains("void assign_t()"), "one process per target:\n{code}");
}

#[test]
fn derived_clocks_fail_closed() {
    // A register on a clock made by combinational logic, by a blocking assignment in an
    // edge-triggered block, or by an edge on an expression would run one delta cycle after
    // the registers on the source clock and sample their new values. Each of these differed
    // from Verilator before the check existed.
    let cases: [(&str, &str, &str); 5] = [
        ("muxed", "module muxed (input logic clk, input logic clk2, input logic sel, input logic [7:0] d, output logic [7:0] q, output logic [7:0] g);\n  logic gclk;\n  logic [7:0] a;\n  assign gclk = sel ? clk2 : clk;\n  always_ff @(posedge clk) a <= d;\n  always_ff @(posedge gclk) g <= a;\n  assign q = a;\nendmodule\n", "combinational logic"),
        ("muxed_inst", "module ckm (input logic clk_i, input logic clk2_i, input logic sel_i, output logic clk_o);\n  assign clk_o = sel_i ? clk2_i : clk_i;\nendmodule\nmodule leaf (input logic ck, input logic [7:0] d, output logic [7:0] q);\n  always_ff @(posedge ck) q <= d;\nendmodule\nmodule muxed_inst (input logic clk, input logic clk2, input logic sel, input logic [7:0] d, output logic [7:0] q, output logic [7:0] g);\n  logic gclk; logic [7:0] a;\n  ckm u_g (.clk_i(clk), .clk2_i(clk2), .sel_i(sel), .clk_o(gclk));\n  always_ff @(posedge clk) a <= d;\n  leaf u_l (.ck(gclk), .d(a), .q(g));\n  assign q = a;\nendmodule\n", "not a register output"),
        // A latch that is transparent while the clock is high is not a gating cell.
        ("highlatch", "module highlatch (input logic clk, input logic en, input logic [7:0] d, output logic [7:0] q, output logic [7:0] g);\n  logic en_l, gclk;\n  logic [7:0] a;\n  always_latch if (clk) en_l = en;\n  assign gclk = clk & en_l;\n  always_ff @(posedge clk) a <= d;\n  always_ff @(posedge gclk) g <= a;\n  assign q = a;\nendmodule\n", "combinational logic"),
        ("divblk", "module divblk (input logic clk, input logic [7:0] d, output logic [7:0] q, output logic [7:0] g);\n  logic div; logic [7:0] a;\n  initial div = 0;\n  always_ff @(posedge clk) div = ~div;\n  always_ff @(posedge clk) a <= d;\n  always_ff @(posedge div) g <= a;\n  assign q = a;\nendmodule\n", "blocking assignment"),
        ("exprck", "module leafv (input logic [1:0] ck, input logic [7:0] d, output logic [7:0] q);\n  always_ff @(posedge ck[0]) q <= d;\nendmodule\nmodule exprck (input logic clk, input logic [7:0] d, output logic [7:0] q, output logic [7:0] g);\n  logic [7:0] a;\n  always_ff @(posedge clk) a <= d;\n  leafv u_l (.ck({1'b0, clk}), .d(a), .q(g));\n  assign q = a;\nendmodule\n", "combinational logic"),
    ];
    for (name, src, why) in cases {
        let err = svcc::sv_rtl_to_sc("t.sv", src, &[])
            .expect_err(&format!("{name}: a derived clock must be rejected"));
        assert!(err.message.contains("clock '"), "{name}: {err}");
        assert!(err.message.contains(why), "{name}: {err}");
    }
}

#[test]
fn clock_gating_cells_become_clock_enables() {
    // IEEE 1800-2023 9.2.2.4 and 9.4.2: a latch transparent while the clock is low, ANDed with
    // the clock, rises with the clock exactly when the enable was set before the edge. The
    // register on the gated clock becomes a register on the root clock with that enable.
    let code = convert(
        "module gated (input logic clk, input logic rst_n, input logic en, input logic [7:0] d, output logic [7:0] g);\n  logic en_l, gclk;\n  always_latch if (!clk) en_l = en;\n  assign gclk = clk & en_l;\n  always_ff @(posedge gclk or negedge rst_n) if (!rst_n) g <= '0; else g <= d;\nendmodule\n",
    );
    assert!(
        code.contains("sensitive << clk.pos() << rst_n.neg();"),
        "{code}"
    );
    // The enable gates the update, the reset is not gated.
    assert!(code.contains("if (en.read() || !rst_n.read())"), "{code}");

    // A register in an instance whose clock port is gated: the instance becomes an enable
    // variant of its module, clocked by the root clock.
    let code = convert(
        "module ckg (input logic clk_i, input logic en_i, output logic clk_o);\n  logic en_l;\n  always_latch if (clk_i == 1'b0) en_l <= en_i;\n  assign clk_o = clk_i & en_l;\nendmodule\nmodule leaf (input logic ck, input logic [7:0] d, output logic [7:0] q);\n  always_ff @(posedge ck) q <= d;\nendmodule\nmodule gated_inst (input logic clk, input logic en, input logic [7:0] d, output logic [7:0] g);\n  logic gclk;\n  ckg u_g (.clk_i(clk), .en_i(en), .clk_o(gclk));\n  leaf u_l (.ck(gclk), .d(d), .q(g));\nendmodule\n",
    );
    assert!(code.contains("SC_MODULE(leaf_ck)"), "{code}");
    assert!(code.contains("u_l.ck(clk);"), "{code}");
    assert!(code.contains("u_l.svcc_cg_ck("), "{code}");
}

#[test]
fn register_driven_clock_and_expression_reset_are_accepted() {
    // A divider register written with nonblocking assignments changes after the update in
    // both languages, and an asynchronous reset taken from a vector bit is read by level.
    let code = convert(
        "module m(input logic clk, input logic [1:0] rn, input logic [7:0] d, output logic [7:0] g);
  logic div;
  always_ff @(posedge clk or negedge rn[0]) if (!rn[0]) div <= 1'b0; else div <= ~div;
  always_ff @(posedge div) g <= d;
endmodule
",
    );
    assert!(code.contains("SC_MODULE"), "{code}");
}

#[test]
fn a_process_writes_back_only_its_own_array_elements() {
    // Element 1 is assigned in pieces, so it goes through a local copy; element 0 belongs
    // to another process and must not be written from here.
    let code = convert(
        "module m(input logic [3:0] a, output logic [3:0] y);\n  logic [3:0] s [2];\n  assign s[0] = a;\n  always_comb begin\n    s[1] = 4'd0;\n    s[1][0] = s[0][3];\n  end\n  assign y = s[1];\nendmodule\n",
    );
    assert!(code.contains("s[1].write(s_v[1]);"), "{code}");
    assert!(!code.contains("s[0].write(s_v[0]);"), "{code}");
    assert!(code.contains("sensitive << s[0];"), "{code}");
}

#[test]
fn multi_file_design_arguments() {
    let dir = std::env::temp_dir().join(format!("svcc-rtl-args-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("inc")).expect("mkdir");
    std::fs::write(dir.join("inc/w.svh"), "`define W 6\n").expect("write");
    std::fs::write(
        dir.join("leaf.sv"),
        "`include \"w.svh\"\nmodule leaf #(parameter int N = 2)(input logic [`W-1:0] a, output logic [`W-1:0] y);\n  assign y = a + N;\nendmodule\n",
    )
    .expect("write");
    std::fs::write(
        dir.join("top.sv"),
        "module top #(parameter int K = 1)(input logic [5:0] a, output logic [5:0] y);\n  leaf #(.N(K)) u (.a(a), .y(y));\nendmodule\n",
    )
    .expect("write");
    let args: Vec<String> = [
        dir.join("leaf.sv").display().to_string(),
        dir.join("top.sv").display().to_string(),
        format!("+incdir+{}", dir.join("inc").display()),
        "-G".into(),
        "K=5".into(),
        "-top".into(),
        "top".into(),
    ]
    .to_vec();
    let (code, design) = svcc::sv_rtl_args_to_sc(&args).expect("convert");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(design.tops, vec!["top".to_string()]);
    assert!(code.contains("sc_dt::sc_uint<6>(a.read() + 5U)"), "{code}");
}
