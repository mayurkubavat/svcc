//! Native `PyO3` Python module (`import svcc`) wrapping the pure-Rust `svcc` compiler & Z3 prover.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

#[pyfunction]
fn sv_to_sc(filename: &str, sv_source: &str) -> PyResult<String> {
    ::svcc::sv_to_sc(filename, sv_source).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Converts synthesizable SystemVerilog RTL to C++17 SystemC on the typed, elaborated path.
/// `tops` names the top modules; empty means every top of the file.
#[pyfunction]
#[pyo3(signature = (filename, sv_source, tops=Vec::new()))]
fn rtl_to_sc(filename: &str, sv_source: &str, tops: Vec<String>) -> PyResult<String> {
    ::svcc::sv_rtl_to_sc(filename, sv_source, &tops)
        .map(|(code, _)| code)
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Proves the SystemC generated for every top module of `sv_source` equal to the source
/// (needs `z3` on PATH). Returns one JSON report per top: `Proved`, `Skipped` or `NotCovered`.
#[pyfunction]
#[pyo3(signature = (filename, sv_source, bound=8))]
fn rtl_prove_json(filename: &str, sv_source: &str, bound: u32) -> PyResult<String> {
    let opts = ::svcc::prove::RtlProveOptions {
        bound,
        ..Default::default()
    };
    let items = ::svcc::prove::prove_rtl_file(filename, sv_source, &opts)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    serde_json::to_string(&items).map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyfunction]
fn sc_to_sv(filename: &str, cpp_source: &str) -> PyResult<String> {
    ::svcc::sc_to_sv(filename, cpp_source).map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyfunction]
fn prove_equivalent_json(
    lhs_filename: &str,
    lhs_source: &str,
    rhs_filename: &str,
    rhs_source: &str,
) -> PyResult<String> {
    let reps = ::svcc::prove_sources_equivalent(lhs_filename, lhs_source, rhs_filename, rhs_source)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    serde_json::to_string(&reps).map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pyfunction]
#[pyo3(signature = (spec_filename, spec_source, imp_filename, imp_source, latency=0))]
fn prove_sec_json(
    spec_filename: &str,
    spec_source: &str,
    imp_filename: &str,
    imp_source: &str,
    latency: u32,
) -> PyResult<String> {
    let opts = ::svcc::prove::SecOptions {
        latency,
        ..Default::default()
    };
    let rep = ::svcc::prove::prove_sec(spec_filename, spec_source, imp_filename, imp_source, &opts)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    serde_json::to_string(&rep).map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pymodule]
fn svcc(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(sv_to_sc, m)?)?;
    m.add_function(wrap_pyfunction!(rtl_to_sc, m)?)?;
    m.add_function(wrap_pyfunction!(rtl_prove_json, m)?)?;
    m.add_function(wrap_pyfunction!(sc_to_sv, m)?)?;
    m.add_function(wrap_pyfunction!(prove_equivalent_json, m)?)?;
    m.add_function(wrap_pyfunction!(prove_sec_json, m)?)?;
    Ok(())
}
