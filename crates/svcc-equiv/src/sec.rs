//! Bridge from SystemVerilog source to the Btor2 transition systems that [`crate::pair`]
//! compares: the `svlang` netlist of one top module, written as word-level Btor2.

#[cfg(feature = "btor2-sec")]
use svcc_ir::SvccError;
#[cfg(feature = "btor2-sec")]
pub use svcc_svlang::Btor2Design;
#[cfg(feature = "btor2-sec")]
use svcc_svlang::Btor2Error;

/// Word-level Btor2 transition system of module `top` in `sv_src`, with the Btor2 symbol of
/// each of its output ports. A design on both edges of its one clock is accepted.
#[cfg(feature = "btor2-sec")]
pub fn sv_to_btor2(sv_src: &str, filename: &str, top: &str) -> Result<Btor2Design, SvccError> {
    svcc_svlang::lower_sv_to_btor2_design(sv_src, filename, top).map_err(|e| match e {
        Btor2Error::Netlist(msg) => SvccError::new(
            "svcc-equiv/netlist",
            format!("svlang netlist lowering failed:\n{}", msg.join("\n")),
        ),
        Btor2Error::Write(e) => {
            SvccError::new("svcc-equiv/btor2", format!("btor2 write failed: {e}"))
        }
    })
}
