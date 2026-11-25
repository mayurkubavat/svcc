//! IEEE Std 1801 (UPF 2.1 / 3.1 / 4.0) Power Intent Intermediate Representation and
//! pure-Rust Tcl/UPF front-end (`svcc-ir::upf`).
//!
//! Designed for 1-to-1 semantic parity with `systemc-lp`'s `sc_lp::upf_design_intent`
//! under Strict Flow A (synthesizable RTL stays 100% power-unaware; all power domains,
//! supply networks, power switches, PSTs, isolation clamps, retention shadow rules,
//! and level shifters live in sidecar `.upf` files) while keeping zero external C or
//! `svlang` dependencies so `InFormal` and `svcc-passes` can embed it directly.

use crate::SvccError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const STAGE: &str = "svcc-ir/upf";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupplyStateKind {
    #[default]
    Off,
    PartialOn,
    FullOn,
    Undetermined,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimState {
    #[default]
    Normal,
    Corrupt,
    CorruptOnActivity,
    CorruptStateOnChange,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClampValue {
    #[default]
    Zero,
    One,
    Latch,
    HighZ,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalSense {
    #[default]
    High,
    Low,
    Posedge,
    Negedge,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupplyResolve {
    #[default]
    Unresolved,
    OneHot,
    Parallel,
    ParallelOneHot,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppliesTo {
    Inputs,
    #[default]
    Outputs,
    Both,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Location {
    #[default]
    SelfLoc,
    Parent,
    Fanout,
    Automatic,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LsRule {
    LowToHigh,
    HighToLow,
    #[default]
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpfObjectKind {
    Inst,
    Port,
    Net,
    Reg,
}

/// Design object descriptor supplied to `[find_objects ...]` during UPF elaboration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfDesignObject {
    pub path: String,
    pub kind: UpfObjectKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfPortState {
    pub name: String,
    pub state: SupplyStateKind,
    pub nom_voltage_uv: i32,
    pub min_voltage_uv: i32,
    pub max_voltage_uv: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfSupplyPort {
    pub name: String,
    pub scope: String,
    pub is_output: bool,
    pub default_on_state: String,
    pub states: BTreeMap<String, UpfPortState>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfSupplyNet {
    pub name: String,
    pub scope: String,
    pub resolve: SupplyResolve,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfSupplyConn {
    pub net_name: String,
    pub port_names: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfPowerStateEntry {
    pub name: String,
    pub supply_expr: String,
    pub logic_expr: String,
    pub simstate: SimState,
    pub has_explicit_simstate: bool,
    pub illegal: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfSupplySet {
    pub name: String,
    pub scope: String,
    pub power_net: String,
    pub ground_net: String,
    pub states: BTreeMap<String, UpfPowerStateEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfPowerDomain {
    pub name: String,
    pub scope: String,
    pub include_scope: bool,
    pub elements: Vec<String>,
    pub primary_supply_set: String,
    pub primary_power_net: String,
    pub primary_ground_net: String,
    pub states: BTreeMap<String, UpfPowerStateEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfSwitchOnState {
    pub state_name: String,
    pub input_supply_port: String,
    pub boolean_expr: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfSwitchOffState {
    pub state_name: String,
    pub boolean_expr: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfPowerSwitch {
    pub name: String,
    pub domain: String,
    pub input_supply_ports: Vec<(String, String)>,
    pub output_supply_port: (String, String),
    pub control_ports: Vec<(String, String)>,
    pub ack_ports: Vec<(String, String)>,
    pub ack_delays_ns: Vec<(String, u64)>,
    pub on_states: Vec<UpfSwitchOnState>,
    pub on_partial_states: Vec<UpfSwitchOnState>,
    pub off_states: Vec<UpfSwitchOffState>,
    pub error_states: Vec<UpfSwitchOffState>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfPst {
    pub name: String,
    pub scope: String,
    pub supplies: Vec<String>,
    pub states: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfIsolation {
    pub name: String,
    pub domain: String,
    pub isolation_supply_set: String,
    pub isolation_power_net: String,
    pub isolation_ground_net: String,
    pub source_supply_set: String,
    pub sink_supply_set: String,
    pub clamp_value: ClampValue,
    pub applies_to: AppliesTo,
    pub location: Location,
    pub has_explicit_location: bool,
    pub diff_supply_only: bool,
    pub no_isolation: bool,
    pub elements: Vec<String>,
    pub exclude_elements: Vec<String>,
    pub isolation_signal: String,
    pub isolation_sense: SignalSense,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfRetention {
    pub name: String,
    pub domain: String,
    pub retention_supply_set: String,
    pub retention_power_net: String,
    pub retention_ground_net: String,
    pub elements: Vec<String>,
    pub exclude_elements: Vec<String>,
    pub save_signal: String,
    pub save_sense: SignalSense,
    pub restore_signal: String,
    pub restore_sense: SignalSense,
    pub save_condition: String,
    pub restore_condition: String,
    pub use_retention_as_primary: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfLevelShifter {
    pub name: String,
    pub domain: String,
    pub source_supply_set: String,
    pub sink_supply_set: String,
    pub applies_to: AppliesTo,
    pub rule: LsRule,
    pub location: Location,
    pub threshold_uv: i32,
    pub elements: Vec<String>,
    pub exclude_elements: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfPortAttribute {
    pub ports: Vec<String>,
    pub domains: Vec<String>,
    pub elements: Vec<String>,
    pub model: String,
    pub applies_to: AppliesTo,
    pub driver_supply: String,
    pub receiver_supply: String,
    pub clamp_value: Option<ClampValue>,
    pub is_isolated: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpfDesignIntent {
    pub upf_version: String,
    pub design_top: String,
    pub domains: BTreeMap<String, UpfPowerDomain>,
    pub supply_ports: BTreeMap<String, UpfSupplyPort>,
    pub supply_nets: BTreeMap<String, UpfSupplyNet>,
    pub connections: Vec<UpfSupplyConn>,
    pub supply_sets: BTreeMap<String, UpfSupplySet>,
    pub power_switches: BTreeMap<String, UpfPowerSwitch>,
    pub psts: BTreeMap<String, UpfPst>,
    pub isolations: BTreeMap<String, UpfIsolation>,
    pub retentions: BTreeMap<String, UpfRetention>,
    pub level_shifters: BTreeMap<String, UpfLevelShifter>,
    pub port_attributes: Vec<UpfPortAttribute>,
}

impl UpfDesignIntent {
    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }

    /// Normalizes a UPF/Verilog/Verilator hierarchical path to dot-separated form relative
    /// to the design root (matching `sc_lp::upf_design_intent::normalize_path`).
    pub fn normalize_path(raw: &str, scope: &str, design_top: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "." {
            return Self::clean_dots(scope, design_top);
        }
        let is_root = trimmed.starts_with('/');
        let body = trimmed.trim_start_matches('/');
        let norm_body = Self::clean_dots(body, design_top);
        if is_root || scope.trim().is_empty() || scope.trim() == "." || scope.trim() == "/" {
            norm_body
        } else {
            let norm_scope = Self::clean_dots(scope, design_top);
            if norm_scope.is_empty() {
                norm_body
            } else if norm_body.is_empty() {
                norm_scope
            } else if norm_body == norm_scope || norm_body.starts_with(&format!("{norm_scope}.")) {
                norm_body
            } else {
                format!("{norm_scope}.{norm_body}")
            }
        }
    }

    fn clean_dots(raw: &str, design_top: &str) -> String {
        let mut s = raw
            .trim()
            .trim_matches('/')
            .replace("__DOT__", ".")
            .replace("__BRA__", "[")
            .replace("__KET__", "]")
            .replace('/', ".");
        while s.contains("..") {
            s = s.replace("..", ".");
        }
        s = s.trim_matches('.').to_string();
        if !design_top.is_empty() {
            if s == design_top {
                return String::new();
            }
            if let Some(rest) = s.strip_prefix(&format!("{design_top}.")) {
                return rest.to_string();
            }
        }
        s
    }

    /// Strips a trailing `[msb:lsb]` or `[bit]` slice from a signal path, returning
    /// `(base_path, Some((msb, lsb)))` when present.
    pub fn strip_bit_slice(path: &str) -> (&str, Option<(u32, u32)>) {
        let Some(open) = path.rfind('[') else {
            return (path, None);
        };
        let Some(inner) = path[open + 1..].strip_suffix(']') else {
            return (path, None);
        };
        if let Some((hi, lo)) = inner.split_once(':') {
            if let (Ok(m), Ok(l)) = (hi.trim().parse::<u32>(), lo.trim().parse::<u32>()) {
                return (&path[..open], Some((m, l)));
            }
        } else if let Ok(b) = inner.trim().parse::<u32>() {
            return (&path[..open], Some((b, b)));
        }
        (path, None)
    }

    /// Wildcard glob matching (`*` matches 0+ chars, `?` matches 1 char).
    pub fn glob_match(pattern: &str, text: &str) -> bool {
        let p: Vec<char> = pattern.chars().collect();
        let t: Vec<char> = text.chars().collect();
        let (mut pi, mut ti) = (0usize, 0usize);
        let (mut star_pi, mut star_ti) = (None, 0usize);
        while ti < t.len() {
            if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
                pi += 1;
                ti += 1;
            } else if pi < p.len() && p[pi] == '*' {
                star_pi = Some(pi);
                pi += 1;
                star_ti = ti;
            } else if let Some(sp) = star_pi {
                pi = sp + 1;
                star_ti += 1;
                ti = star_ti;
            } else {
                return false;
            }
        }
        while pi < p.len() && p[pi] == '*' {
            pi += 1;
        }
        pi == p.len()
    }

    /// Resolves the deepest matching UPF power domain for a normalized design path
    /// (`""` for the root module, `"u_timer"` or `"u_timer.cfg_q"` for children).
    pub fn find_domain_for_path(&self, path: &str) -> Option<&str> {
        let mut best_elem: Option<(&str, usize)> = None;
        for dom in self.domains.values() {
            for el in &dom.elements {
                let matched = if el.contains('*') || el.contains('?') {
                    Self::glob_match(el, path)
                } else {
                    path == el || path.starts_with(&format!("{el}."))
                };
                if matched {
                    let len = el.len();
                    if best_elem.is_none_or(|(_, b)| len > b) {
                        best_elem = Some((dom.name.as_str(), len));
                    }
                }
            }
        }
        if let Some((d, _)) = best_elem {
            return Some(d);
        }

        let mut best_scope: Option<(&str, usize)> = None;
        for dom in self.domains.values() {
            if !dom.include_scope {
                continue;
            }
            let sc = dom.scope.as_str();
            let matched = sc.is_empty() || path == sc || path.starts_with(&format!("{sc}."));
            if matched {
                let len = sc.len();
                if best_scope.is_none_or(|(_, b)| len > b) {
                    best_scope = Some((dom.name.as_str(), len));
                }
            }
        }
        best_scope.map(|(d, _)| d)
    }

    /// Resolves the primary power net name of domain `dom_name`.
    pub fn domain_power_net(&self, dom_name: &str) -> Option<&str> {
        let dom = self.domains.get(dom_name)?;
        if !dom.primary_power_net.is_empty() {
            return Some(dom.primary_power_net.as_str());
        }
        if !dom.primary_supply_set.is_empty() {
            if let Some(ss) = self.supply_sets.get(&dom.primary_supply_set) {
                if !ss.power_net.is_empty() {
                    return Some(ss.power_net.as_str());
                }
            }
        }
        None
    }

    /// Resolves the power net name of a supply set (or direct net fallback).
    pub fn supply_set_power_net<'a>(&'a self, ss_or_net: &'a str) -> Option<&'a str> {
        if ss_or_net.is_empty() {
            return None;
        }
        if let Some(ss) = self.supply_sets.get(ss_or_net) {
            if !ss.power_net.is_empty() {
                return Some(ss.power_net.as_str());
            }
        }
        if self.supply_nets.contains_key(ss_or_net) {
            return Some(ss_or_net);
        }
        None
    }
}

// ============================================================================
// Pure-Rust IEEE 1801 UPF / Tcl Subset Evaluator
// ============================================================================

/// Parses an IEEE 1801 `.upf` file from disk, resolving `load_upf` relative to its directory.
pub fn parse_upf_file(
    path: &Path,
    design_objects: &[UpfDesignObject],
) -> Result<UpfDesignIntent, SvccError> {
    let src = fs::read_to_string(path)
        .map_err(|e| SvccError::new(STAGE, format!("cannot read {}: {e}", path.display())))?;
    parse_upf_str(&src, path.parent(), design_objects)
}

/// Parses IEEE 1801 UPF source text into [`UpfDesignIntent`].
pub fn parse_upf_str(
    src: &str,
    base_dir: Option<&Path>,
    design_objects: &[UpfDesignObject],
) -> Result<UpfDesignIntent, SvccError> {
    let mut interp = UpfInterp {
        intent: UpfDesignIntent {
            upf_version: "3.1".to_string(),
            ..Default::default()
        },
        vars: BTreeMap::new(),
        scope: String::new(),
        base_dir: base_dir.map(Path::to_path_buf),
        objects: design_objects,
    };
    interp.eval_script(src)?;
    interp.finalize_supply_links();
    Ok(interp.intent)
}

struct UpfInterp<'a> {
    intent: UpfDesignIntent,
    vars: BTreeMap<String, String>,
    scope: String,
    base_dir: Option<std::path::PathBuf>,
    objects: &'a [UpfDesignObject],
}

impl<'a> UpfInterp<'a> {
    fn norm(&self, raw: &str) -> String {
        UpfDesignIntent::normalize_path(raw, &self.scope, &self.intent.design_top)
    }

    fn finalize_supply_links(&mut self) {
        // Ensure implicit supply nets exist for connected supply ports and switch outputs.
        let conns = self.intent.connections.clone();
        for c in &conns {
            self.intent
                .supply_nets
                .entry(c.net_name.clone())
                .or_insert_with(|| UpfSupplyNet {
                    name: c.net_name.clone(),
                    scope: String::new(),
                    resolve: SupplyResolve::Unresolved,
                });
        }
        let switches: Vec<_> = self.intent.power_switches.values().cloned().collect();
        for sw in switches {
            let (_, out_net) = &sw.output_supply_port;
            if !out_net.is_empty() {
                self.intent
                    .supply_nets
                    .entry(out_net.clone())
                    .or_insert_with(|| UpfSupplyNet {
                        name: out_net.clone(),
                        scope: String::new(),
                        resolve: SupplyResolve::Unresolved,
                    });
            }
        }
        // Populate domain primary_power_net / ground_net from primary_supply_set when unset.
        let ssets = self.intent.supply_sets.clone();
        for dom in self.intent.domains.values_mut() {
            if dom.primary_power_net.is_empty() && !dom.primary_supply_set.is_empty() {
                if let Some(ss) = ssets.get(&dom.primary_supply_set) {
                    dom.primary_power_net = ss.power_net.clone();
                    dom.primary_ground_net = ss.ground_net.clone();
                }
            }
        }
    }

    fn eval_script(&mut self, script: &str) -> Result<String, SvccError> {
        let stmts = split_tcl_statements(script)?;
        let mut last = String::new();
        for raw_words in stmts {
            let mut words = Vec::with_capacity(raw_words.len());
            for w in raw_words {
                words.push(self.expand_word(&w)?);
            }
            if words.is_empty() {
                continue;
            }
            last = self.exec_cmd(&words)?;
        }
        Ok(last)
    }

    fn expand_word(&mut self, raw: &TclWord) -> Result<String, SvccError> {
        match raw {
            TclWord::Braced(s) => Ok(s.clone()),
            TclWord::Bare(s) | TclWord::Quoted(s) => self.subst_str(s),
        }
    }

    fn subst_str(&mut self, s: &str) -> Result<String, SvccError> {
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '\\' if i + 1 < chars.len() => {
                    i += 1;
                    out.push(chars[i]);
                    i += 1;
                }
                '$' => {
                    i += 1;
                    if i < chars.len() && chars[i] == '{' {
                        i += 1;
                        let start = i;
                        while i < chars.len() && chars[i] != '}' {
                            i += 1;
                        }
                        let name: String = chars[start..i].iter().collect();
                        if i < chars.len() {
                            i += 1;
                        }
                        let v = self.vars.get(&name).cloned().ok_or_else(|| {
                            SvccError::new(STAGE, format!("undefined Tcl variable '${{{name}}}'"))
                        })?;
                        out.push_str(&v);
                    } else {
                        let start = i;
                        while i < chars.len()
                            && (chars[i].is_ascii_alphanumeric() || chars[i] == '_')
                        {
                            i += 1;
                        }
                        if start == i {
                            out.push('$');
                        } else {
                            let name: String = chars[start..i].iter().collect();
                            let v = self.vars.get(&name).cloned().ok_or_else(|| {
                                SvccError::new(STAGE, format!("undefined Tcl variable '${name}'"))
                            })?;
                            out.push_str(&v);
                        }
                    }
                }
                '[' => {
                    let (cmd_src, next_i) = slice_balanced(&chars, i, '[', ']')?;
                    let res = self.eval_script(&cmd_src)?;
                    out.push_str(&res);
                    i = next_i;
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        Ok(out)
    }

    fn exec_cmd(&mut self, args: &[String]) -> Result<String, SvccError> {
        let cmd = args[0].as_str();
        match cmd {
            "upf_version" => {
                if let Some(v) = args.get(1) {
                    self.intent.upf_version = v.clone();
                }
                Ok(self.intent.upf_version.clone())
            }
            "set" => match args.len() {
                2 => self.vars.get(&args[1]).cloned().ok_or_else(|| {
                    SvccError::new(STAGE, format!("undefined Tcl variable '{}'", args[1]))
                }),
                3 => {
                    self.vars.insert(args[1].clone(), args[2].clone());
                    Ok(args[2].clone())
                }
                _ => Err(SvccError::new(STAGE, "usage: set var ?value?")),
            },
            "foreach" => {
                if args.len() != 4 {
                    return Err(SvccError::new(STAGE, "usage: foreach var list body"));
                }
                let var = &args[1];
                let items = split_tcl_list(&args[2])?;
                let body = &args[3];
                let mut last = String::new();
                for item in items {
                    self.vars.insert(var.clone(), item);
                    last = self.eval_script(body)?;
                }
                Ok(last)
            }
            "set_design_top" => {
                let raw = args
                    .get(1)
                    .ok_or_else(|| SvccError::new(STAGE, "set_design_top requires a name"))?;
                let top = raw
                    .trim_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(raw)
                    .to_string();
                self.intent.design_top = top.clone();
                Ok(top)
            }
            "set_scope" => {
                let raw = args.get(1).map(String::as_str).unwrap_or(".");
                if raw == "." || raw == "/" {
                    self.scope.clear();
                } else {
                    self.scope =
                        UpfDesignIntent::normalize_path(raw, &self.scope, &self.intent.design_top);
                }
                Ok(self.scope.clone())
            }
            "load_upf" => self.cmd_load_upf(&args[1..]),
            "find_objects" => self.cmd_find_objects(&args[1..]),
            "create_supply_port" => self.cmd_create_supply_port(&args[1..]),
            "add_port_state" => self.cmd_add_port_state(&args[1..]),
            "create_supply_net" => self.cmd_create_supply_net(&args[1..]),
            "connect_supply_net" => self.cmd_connect_supply_net(&args[1..]),
            "create_supply_set" => self.cmd_create_supply_set(&args[1..]),
            "associate_supply_set" => self.cmd_associate_supply_set(&args[1..]),
            "create_power_domain" => self.cmd_create_power_domain(&args[1..]),
            "add_power_state" => self.cmd_add_power_state(&args[1..]),
            "create_power_switch" => self.cmd_create_power_switch(&args[1..]),
            "create_pst" => self.cmd_create_pst(&args[1..]),
            "add_pst_state" => self.cmd_add_pst_state(&args[1..]),
            "set_isolation" => self.cmd_set_isolation(&args[1..]),
            "set_isolation_control" => self.cmd_set_isolation_control(&args[1..]),
            "set_retention" => self.cmd_set_retention(&args[1..]),
            "set_retention_control" => self.cmd_set_retention_control(&args[1..]),
            "set_level_shifter" => self.cmd_set_level_shifter(&args[1..]),
            "set_port_attributes" => self.cmd_set_port_attributes(&args[1..]),
            other => Err(SvccError::new(
                STAGE,
                format!("unsupported UPF/Tcl command '{other}'"),
            )),
        }
    }

    fn cmd_load_upf(&mut self, args: &[String]) -> Result<String, SvccError> {
        let mut file = None;
        let mut sub_scope = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-scope" => {
                    sub_scope = args.get(i + 1).cloned();
                    i += 2;
                }
                "-version" => {
                    i += 2;
                }
                s if !s.starts_with('-') => {
                    file = Some(s.to_string());
                    i += 1;
                }
                other => {
                    return Err(SvccError::new(
                        STAGE,
                        format!("load_upf: unknown option '{other}'"),
                    ))
                }
            }
        }
        let rel = file.ok_or_else(|| SvccError::new(STAGE, "load_upf missing file path"))?;
        let resolved = match &self.base_dir {
            Some(dir) if Path::new(&rel).is_relative() => dir.join(&rel),
            _ => std::path::PathBuf::from(&rel),
        };
        let sub_src = fs::read_to_string(&resolved)
            .map_err(|e| SvccError::new(STAGE, format!("load_upf {}: {e}", resolved.display())))?;
        let saved_scope = self.scope.clone();
        let saved_dir = self.base_dir.clone();
        if let Some(sc) = sub_scope {
            self.scope = self.norm(&sc);
        }
        self.base_dir = resolved.parent().map(Path::to_path_buf);
        let res = self.eval_script(&sub_src);
        self.scope = saved_scope;
        self.base_dir = saved_dir;
        res
    }

    fn cmd_find_objects(&self, args: &[String]) -> Result<String, SvccError> {
        let mut search_scope = self.scope.clone();
        let mut pattern = "*".to_string();
        let mut obj_type = None::<String>;
        let mut transitive = true;
        let mut i = 0;
        if let Some(first) = args.first() {
            if !first.starts_with('-') {
                search_scope = self.norm(first);
                i = 1;
            }
        }
        while i < args.len() {
            match args[i].as_str() {
                "-pattern" => {
                    pattern = args.get(i + 1).cloned().unwrap_or_else(|| "*".into());
                    i += 2;
                }
                "-object_type" => {
                    obj_type = args.get(i + 1).map(|s| s.to_ascii_lowercase());
                    i += 2;
                }
                "-transitive" => {
                    if let Some(v) = args.get(i + 1) {
                        transitive =
                            !matches!(v.to_ascii_lowercase().as_str(), "false" | "0" | "no");
                        i += 2;
                    } else {
                        transitive = true;
                        i += 1;
                    }
                }
                "-ignore_case" | "-exact" => {
                    i += 1;
                }
                _ => i += 1,
            }
        }
        let norm_pat = pattern.replace('/', ".");
        let mut matches = Vec::new();
        for obj in self.objects {
            if let Some(ot) = &obj_type {
                let ok_kind = match ot.as_str() {
                    "inst" | "instance" | "model" => obj.kind == UpfObjectKind::Inst,
                    "port" => obj.kind == UpfObjectKind::Port,
                    "net" | "signal" => matches!(obj.kind, UpfObjectKind::Net | UpfObjectKind::Reg),
                    "reg" | "register" => {
                        matches!(obj.kind, UpfObjectKind::Reg | UpfObjectKind::Net)
                    }
                    _ => true,
                };
                if !ok_kind {
                    continue;
                }
            }
            let rel = if search_scope.is_empty() {
                obj.path.as_str()
            } else if obj.path == search_scope {
                ""
            } else if let Some(r) = obj.path.strip_prefix(&format!("{search_scope}.")) {
                r
            } else {
                continue;
            };
            if !transitive && rel.contains('.') {
                continue;
            }
            let leaf = rel.rsplit('.').next().unwrap_or(rel);
            if UpfDesignIntent::glob_match(&norm_pat, rel)
                || UpfDesignIntent::glob_match(&norm_pat, &obj.path)
                || (!norm_pat.contains('.') && UpfDesignIntent::glob_match(&norm_pat, leaf))
            {
                matches.push(obj.path.clone());
            }
        }
        Ok(matches.join(" "))
    }

    fn cmd_create_supply_port(&mut self, args: &[String]) -> Result<String, SvccError> {
        let raw_name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "create_supply_port missing port name"))?;
        let mut domain_scope = self.scope.clone();
        let mut is_output = false;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-domain" => {
                    if let Some(d) = args.get(i + 1) {
                        if let Some(dom) = self.intent.domains.get(d) {
                            domain_scope = dom.scope.clone();
                        }
                    }
                    i += 2;
                }
                "-direction" => {
                    is_output = args
                        .get(i + 1)
                        .is_some_and(|d| d.eq_ignore_ascii_case("out"));
                    i += 2;
                }
                _ => i += 1,
            }
        }
        let name =
            UpfDesignIntent::normalize_path(raw_name, &domain_scope, &self.intent.design_top);
        self.intent.supply_ports.insert(
            name.clone(),
            UpfSupplyPort {
                name: name.clone(),
                scope: domain_scope,
                is_output,
                default_on_state: String::new(),
                states: BTreeMap::new(),
            },
        );
        Ok(name)
    }

    fn cmd_add_port_state(&mut self, args: &[String]) -> Result<String, SvccError> {
        let raw_port = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "add_port_state missing port name"))?;
        let port_name = self.norm(raw_port);
        let port = self
            .intent
            .supply_ports
            .entry(port_name.clone())
            .or_insert_with(|| UpfSupplyPort {
                name: port_name.clone(),
                scope: self.scope.clone(),
                ..Default::default()
            });
        let mut i = 1;
        while i < args.len() {
            if args[i] == "-state" {
                let spec = args
                    .get(i + 1)
                    .ok_or_else(|| SvccError::new(STAGE, "add_port_state -state missing value"))?;
                let items = split_tcl_list(spec)?;
                if items.len() >= 2 {
                    let st_name = items[0].clone();
                    let val = items[1].to_ascii_lowercase();
                    let ps = if val == "off" {
                        UpfPortState {
                            name: st_name.clone(),
                            state: SupplyStateKind::Off,
                            nom_voltage_uv: 0,
                            min_voltage_uv: 0,
                            max_voltage_uv: 0,
                        }
                    } else if items.len() >= 4 {
                        let min_uv = parse_voltage_uv(&items[1])?;
                        let nom_uv = parse_voltage_uv(&items[2])?;
                        let max_uv = parse_voltage_uv(&items[3])?;
                        UpfPortState {
                            name: st_name.clone(),
                            state: SupplyStateKind::FullOn,
                            nom_voltage_uv: nom_uv,
                            min_voltage_uv: min_uv,
                            max_voltage_uv: max_uv,
                        }
                    } else {
                        let uv = parse_voltage_uv(&items[1])?;
                        UpfPortState {
                            name: st_name.clone(),
                            state: SupplyStateKind::FullOn,
                            nom_voltage_uv: uv,
                            min_voltage_uv: uv,
                            max_voltage_uv: uv,
                        }
                    };
                    if ps.state == SupplyStateKind::FullOn && port.default_on_state.is_empty() {
                        port.default_on_state = st_name.clone();
                    }
                    port.states.insert(st_name, ps);
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        Ok(port_name)
    }

    fn cmd_create_supply_net(&mut self, args: &[String]) -> Result<String, SvccError> {
        let raw_name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "create_supply_net missing net name"))?;
        let mut scope = self.scope.clone();
        let mut resolve = SupplyResolve::Unresolved;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-domain" => {
                    if let Some(d) = args.get(i + 1) {
                        if let Some(dom) = self.intent.domains.get(d) {
                            scope = dom.scope.clone();
                        }
                    }
                    i += 2;
                }
                "-resolve" => {
                    resolve = match args.get(i + 1).map(String::as_str).unwrap_or("unresolved") {
                        "one_hot" => SupplyResolve::OneHot,
                        "parallel" => SupplyResolve::Parallel,
                        "parallel_one_hot" => SupplyResolve::ParallelOneHot,
                        _ => SupplyResolve::Unresolved,
                    };
                    i += 2;
                }
                "-reuse" => i += 1,
                _ => i += 1,
            }
        }
        let name = UpfDesignIntent::normalize_path(raw_name, &scope, &self.intent.design_top);
        self.intent
            .supply_nets
            .entry(name.clone())
            .and_modify(|n| n.resolve = resolve)
            .or_insert(UpfSupplyNet {
                name: name.clone(),
                scope,
                resolve,
            });
        Ok(name)
    }

    fn cmd_connect_supply_net(&mut self, args: &[String]) -> Result<String, SvccError> {
        let raw_net = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "connect_supply_net missing net"))?;
        let net_name = self.norm(raw_net);
        let mut ports = Vec::new();
        let mut i = 1;
        while i < args.len() {
            if args[i] == "-ports" {
                if let Some(list) = args.get(i + 1) {
                    for p in split_tcl_list(list)? {
                        ports.push(self.norm(&p));
                    }
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        self.intent.connections.push(UpfSupplyConn {
            net_name: net_name.clone(),
            port_names: ports,
        });
        Ok(net_name)
    }

    fn cmd_create_supply_set(&mut self, args: &[String]) -> Result<String, SvccError> {
        let raw_name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "create_supply_set missing name"))?;
        let name = self.norm(raw_name);
        let scope = self.scope.clone();
        let mut power_net = None;
        let mut ground_net = None;
        let mut i = 1;
        while i < args.len() {
            if args[i] == "-function" {
                if let Some(spec) = args.get(i + 1) {
                    let items = split_tcl_list(spec)?;
                    if items.len() >= 2 {
                        let net = self.norm(&items[1]);
                        match items[0].as_str() {
                            "power" => power_net = Some(net),
                            "ground" => ground_net = Some(net),
                            _ => {}
                        }
                    }
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        let ss = self
            .intent
            .supply_sets
            .entry(name.clone())
            .or_insert_with(|| UpfSupplySet {
                name: name.clone(),
                scope,
                ..Default::default()
            });
        if let Some(p) = power_net {
            ss.power_net = p;
        }
        if let Some(g) = ground_net {
            ss.ground_net = g;
        }
        Ok(name)
    }

    fn cmd_associate_supply_set(&mut self, args: &[String]) -> Result<String, SvccError> {
        let raw_ss = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "associate_supply_set missing supply set"))?;
        let ss = self.norm(raw_ss);
        let mut i = 1;
        while i < args.len() {
            if args[i] == "-handle" {
                if let Some(h) = args.get(i + 1) {
                    if let Some((dom_name, handle)) = h.split_once('.') {
                        if let Some(dom) = self.intent.domains.get_mut(dom_name) {
                            if handle == "primary" {
                                dom.primary_supply_set = ss.clone();
                            }
                        }
                    }
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        Ok(ss)
    }

    fn cmd_create_power_domain(&mut self, args: &[String]) -> Result<String, SvccError> {
        let name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "create_power_domain missing domain name"))?
            .clone();
        let mut dom = UpfPowerDomain {
            name: name.clone(),
            scope: self.scope.clone(),
            ..Default::default()
        };
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-include_scope" => {
                    dom.include_scope = true;
                    i += 1;
                }
                "-scope" => {
                    if let Some(s) = args.get(i + 1) {
                        dom.scope = self.norm(s);
                    }
                    i += 2;
                }
                "-elements" => {
                    if let Some(list) = args.get(i + 1) {
                        for el in split_tcl_list(list)? {
                            if el == "." {
                                dom.include_scope = true;
                            } else {
                                dom.elements.push(self.norm(&el));
                            }
                        }
                    }
                    i += 2;
                }
                "-supply" => {
                    if let Some(spec) = args.get(i + 1) {
                        let items = split_tcl_list(spec)?;
                        if items.len() >= 2 && items[0] == "primary" {
                            dom.primary_supply_set = self.norm(&items[1]);
                        }
                    }
                    i += 2;
                }
                _ => i += 1,
            }
        }
        self.intent.domains.insert(name.clone(), dom);
        Ok(name)
    }

    fn cmd_add_power_state(&mut self, args: &[String]) -> Result<String, SvccError> {
        let raw_obj = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "add_power_state missing object"))?;
        let mut is_domain = false;
        let mut is_supply = false;
        let mut entries = Vec::new();
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-domain" => {
                    is_domain = true;
                    i += 1;
                }
                "-supply" => {
                    is_supply = true;
                    i += 1;
                }
                "-update" => i += 1,
                "-state" => {
                    if let Some(spec) = args.get(i + 1) {
                        entries.push(parse_power_state_entry(spec)?);
                    }
                    i += 2;
                }
                _ => i += 1,
            }
        }
        let obj_norm = self.norm(raw_obj);
        if is_domain || (!is_supply && self.intent.domains.contains_key(raw_obj)) {
            let dom = self.intent.domains.get_mut(raw_obj).ok_or_else(|| {
                SvccError::new(
                    STAGE,
                    format!("add_power_state: unknown domain '{raw_obj}'"),
                )
            })?;
            for e in entries {
                dom.states.insert(e.name.clone(), e);
            }
        } else {
            let ss_key = if let Some((dom_name, "primary")) = raw_obj.split_once('.') {
                self.intent
                    .domains
                    .get(dom_name)
                    .map(|d| d.primary_supply_set.clone())
                    .unwrap_or(obj_norm)
            } else {
                obj_norm
            };
            let ss = self
                .intent
                .supply_sets
                .entry(ss_key.clone())
                .or_insert_with(|| UpfSupplySet {
                    name: ss_key,
                    scope: self.scope.clone(),
                    ..Default::default()
                });
            for e in entries {
                ss.states.insert(e.name.clone(), e);
            }
        }
        Ok(raw_obj.clone())
    }

    fn cmd_create_power_switch(&mut self, args: &[String]) -> Result<String, SvccError> {
        let raw_name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "create_power_switch missing switch name"))?;
        let name = self.norm(raw_name);
        let mut sw = UpfPowerSwitch {
            name: name.clone(),
            ..Default::default()
        };
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-domain" => {
                    sw.domain = args.get(i + 1).cloned().unwrap_or_default();
                    i += 2;
                }
                "-input_supply_port" => {
                    if let Some(spec) = args.get(i + 1) {
                        let items = split_tcl_list(spec)?;
                        if items.len() >= 2 {
                            sw.input_supply_ports
                                .push((items[0].clone(), self.norm(&items[1])));
                        }
                    }
                    i += 2;
                }
                "-output_supply_port" => {
                    if let Some(spec) = args.get(i + 1) {
                        let items = split_tcl_list(spec)?;
                        if items.len() >= 2 {
                            let port_path = format!("{name}.{}", items[0]);
                            let net = self.norm(&items[1]);
                            sw.output_supply_port = (items[0].clone(), net.clone());
                            self.intent.supply_ports.entry(port_path.clone()).or_insert(
                                UpfSupplyPort {
                                    name: port_path.clone(),
                                    scope: self.scope.clone(),
                                    is_output: true,
                                    ..Default::default()
                                },
                            );
                            self.intent.connections.push(UpfSupplyConn {
                                net_name: net,
                                port_names: vec![port_path],
                            });
                        }
                    }
                    i += 2;
                }
                "-control_port" => {
                    if let Some(spec) = args.get(i + 1) {
                        let items = split_tcl_list(spec)?;
                        if items.len() >= 2 {
                            sw.control_ports
                                .push((items[0].clone(), self.norm(&items[1])));
                        }
                    }
                    i += 2;
                }
                "-ack_port" => {
                    if let Some(spec) = args.get(i + 1) {
                        let items = split_tcl_list(spec)?;
                        if items.len() >= 2 {
                            sw.ack_ports.push((items[0].clone(), self.norm(&items[1])));
                        }
                    }
                    i += 2;
                }
                "-ack_delay" => {
                    if let Some(spec) = args.get(i + 1) {
                        let items = split_tcl_list(spec)?;
                        if items.len() >= 2 {
                            let ns = items[1].parse::<f64>().unwrap_or(0.0).max(0.0) as u64;
                            sw.ack_delays_ns.push((items[0].clone(), ns));
                        }
                    }
                    i += 2;
                }
                "-on_state" | "-on_partial_state" => {
                    let partial = args[i] == "-on_partial_state";
                    if let Some(spec) = args.get(i + 1) {
                        let items = split_tcl_list(spec)?;
                        if items.len() >= 3 {
                            let entry = UpfSwitchOnState {
                                state_name: items[0].clone(),
                                input_supply_port: items[1].clone(),
                                boolean_expr: items[2].clone(),
                            };
                            if partial {
                                sw.on_partial_states.push(entry);
                            } else {
                                sw.on_states.push(entry);
                            }
                        }
                    }
                    i += 2;
                }
                "-off_state" | "-error_state" => {
                    let is_err = args[i] == "-error_state";
                    if let Some(spec) = args.get(i + 1) {
                        let items = split_tcl_list(spec)?;
                        if items.len() >= 2 {
                            let entry = UpfSwitchOffState {
                                state_name: items[0].clone(),
                                boolean_expr: items[1].clone(),
                            };
                            if is_err {
                                sw.error_states.push(entry);
                            } else {
                                sw.off_states.push(entry);
                            }
                        }
                    }
                    i += 2;
                }
                _ => i += 1,
            }
        }
        self.intent.power_switches.insert(name.clone(), sw);
        Ok(name)
    }

    fn cmd_create_pst(&mut self, args: &[String]) -> Result<String, SvccError> {
        let name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "create_pst missing name"))?
            .clone();
        let mut supplies = Vec::new();
        let mut i = 1;
        while i < args.len() {
            if args[i] == "-supplies" {
                if let Some(list) = args.get(i + 1) {
                    for s in split_tcl_list(list)? {
                        supplies.push(self.norm(&s));
                    }
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        self.intent.psts.insert(
            name.clone(),
            UpfPst {
                name: name.clone(),
                scope: self.scope.clone(),
                supplies,
                states: BTreeMap::new(),
            },
        );
        Ok(name)
    }

    fn cmd_add_pst_state(&mut self, args: &[String]) -> Result<String, SvccError> {
        let state_name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "add_pst_state missing state name"))?
            .clone();
        let mut pst_name = String::new();
        let mut tuple = Vec::new();
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-pst" => {
                    pst_name = args.get(i + 1).cloned().unwrap_or_default();
                    i += 2;
                }
                "-state" => {
                    if let Some(list) = args.get(i + 1) {
                        tuple = split_tcl_list(list)?;
                    }
                    i += 2;
                }
                _ => i += 1,
            }
        }
        let pst = self.intent.psts.get_mut(&pst_name).ok_or_else(|| {
            SvccError::new(STAGE, format!("add_pst_state: unknown PST '{pst_name}'"))
        })?;
        pst.states.insert(state_name.clone(), tuple);
        Ok(state_name)
    }

    fn cmd_set_isolation(&mut self, args: &[String]) -> Result<String, SvccError> {
        let name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "set_isolation missing strategy name"))?
            .clone();
        let mut iso = self
            .intent
            .isolations
            .remove(&name)
            .unwrap_or_else(|| UpfIsolation {
                name: name.clone(),
                diff_supply_only: true,
                ..Default::default()
            });
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-domain" => {
                    iso.domain = args.get(i + 1).cloned().unwrap_or_default();
                    i += 2;
                }
                "-isolation_supply_set" => {
                    if let Some(v) = args.get(i + 1) {
                        iso.isolation_supply_set = self.norm(v);
                    }
                    i += 2;
                }
                "-isolation_power_net" => {
                    if let Some(v) = args.get(i + 1) {
                        iso.isolation_power_net = self.norm(v);
                    }
                    i += 2;
                }
                "-isolation_ground_net" => {
                    if let Some(v) = args.get(i + 1) {
                        iso.isolation_ground_net = self.norm(v);
                    }
                    i += 2;
                }
                "-source" => {
                    if let Some(v) = args.get(i + 1) {
                        iso.source_supply_set = self.norm(v);
                    }
                    i += 2;
                }
                "-sink" => {
                    if let Some(v) = args.get(i + 1) {
                        iso.sink_supply_set = self.norm(v);
                    }
                    i += 2;
                }
                "-clamp_value" => {
                    iso.clamp_value =
                        parse_clamp(args.get(i + 1).map(String::as_str).unwrap_or("0"));
                    i += 2;
                }
                "-applies_to" => {
                    iso.applies_to =
                        parse_applies_to(args.get(i + 1).map(String::as_str).unwrap_or("outputs"));
                    i += 2;
                }
                "-location" => {
                    iso.location =
                        parse_location(args.get(i + 1).map(String::as_str).unwrap_or("self"));
                    iso.has_explicit_location = true;
                    i += 2;
                }
                "-diff_supply_only" => {
                    iso.diff_supply_only = !matches!(
                        args.get(i + 1).map(|s| s.to_ascii_lowercase()).as_deref(),
                        Some("false" | "0" | "no")
                    );
                    i += 2;
                }
                "-no_isolation" => {
                    iso.no_isolation = true;
                    i += 1;
                }
                "-elements" => {
                    if let Some(list) = args.get(i + 1) {
                        for el in split_tcl_list(list)? {
                            iso.elements.push(self.norm(&el));
                        }
                    }
                    i += 2;
                }
                "-exclude_elements" => {
                    if let Some(list) = args.get(i + 1) {
                        for el in split_tcl_list(list)? {
                            iso.exclude_elements.push(self.norm(&el));
                        }
                    }
                    i += 2;
                }
                "-isolation_signal" => {
                    if let Some(v) = args.get(i + 1) {
                        iso.isolation_signal = self.norm(v);
                    }
                    i += 2;
                }
                "-isolation_sense" => {
                    iso.isolation_sense =
                        parse_sense(args.get(i + 1).map(String::as_str).unwrap_or("high"));
                    i += 2;
                }
                "-update" => i += 1,
                _ => i += 1,
            }
        }
        self.intent.isolations.insert(name.clone(), iso);
        Ok(name)
    }

    fn cmd_set_isolation_control(&mut self, args: &[String]) -> Result<String, SvccError> {
        let name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "set_isolation_control missing name"))?
            .clone();
        let mut domain = None;
        let mut sig = None;
        let mut sense = None;
        let mut loc = None;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-domain" => {
                    domain = args.get(i + 1).cloned();
                    i += 2;
                }
                "-isolation_signal" => {
                    sig = args.get(i + 1).map(|v| self.norm(v));
                    i += 2;
                }
                "-isolation_sense" => {
                    sense = args.get(i + 1).map(|v| parse_sense(v));
                    i += 2;
                }
                "-location" => {
                    loc = args.get(i + 1).map(|v| parse_location(v));
                    i += 2;
                }
                _ => i += 1,
            }
        }
        let iso = self
            .intent
            .isolations
            .entry(name.clone())
            .or_insert_with(|| UpfIsolation {
                name: name.clone(),
                diff_supply_only: true,
                ..Default::default()
            });
        if let Some(d) = domain {
            iso.domain = d;
        }
        if let Some(s) = sig {
            iso.isolation_signal = s;
        }
        if let Some(sn) = sense {
            iso.isolation_sense = sn;
        }
        if let Some(l) = loc {
            iso.location = l;
            iso.has_explicit_location = true;
        }
        Ok(name)
    }

    fn cmd_set_retention(&mut self, args: &[String]) -> Result<String, SvccError> {
        let name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "set_retention missing strategy name"))?
            .clone();
        let mut ret = self
            .intent
            .retentions
            .remove(&name)
            .unwrap_or_else(|| UpfRetention {
                name: name.clone(),
                ..Default::default()
            });
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-domain" => {
                    ret.domain = args.get(i + 1).cloned().unwrap_or_default();
                    i += 2;
                }
                "-retention_supply_set" => {
                    if let Some(v) = args.get(i + 1) {
                        ret.retention_supply_set = self.norm(v);
                    }
                    i += 2;
                }
                "-retention_power_net" => {
                    if let Some(v) = args.get(i + 1) {
                        ret.retention_power_net = self.norm(v);
                    }
                    i += 2;
                }
                "-retention_ground_net" => {
                    if let Some(v) = args.get(i + 1) {
                        ret.retention_ground_net = self.norm(v);
                    }
                    i += 2;
                }
                "-elements" => {
                    if let Some(list) = args.get(i + 1) {
                        for el in split_tcl_list(list)? {
                            ret.elements.push(self.norm(&el));
                        }
                    }
                    i += 2;
                }
                "-exclude_elements" => {
                    if let Some(list) = args.get(i + 1) {
                        for el in split_tcl_list(list)? {
                            ret.exclude_elements.push(self.norm(&el));
                        }
                    }
                    i += 2;
                }
                "-save_signal" => {
                    if let Some(spec) = args.get(i + 1) {
                        let (s, sn) = self.parse_sig_sense_pair(spec)?;
                        ret.save_signal = s;
                        ret.save_sense = sn;
                    }
                    i += 2;
                }
                "-restore_signal" => {
                    if let Some(spec) = args.get(i + 1) {
                        let (s, sn) = self.parse_sig_sense_pair(spec)?;
                        ret.restore_signal = s;
                        ret.restore_sense = sn;
                    }
                    i += 2;
                }
                "-use_retention_as_primary" => {
                    ret.use_retention_as_primary = true;
                    i += 1;
                }
                "-update" => i += 1,
                _ => i += 1,
            }
        }
        self.intent.retentions.insert(name.clone(), ret);
        Ok(name)
    }

    fn cmd_set_retention_control(&mut self, args: &[String]) -> Result<String, SvccError> {
        let name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "set_retention_control missing name"))?
            .clone();
        let mut domain = None;
        let mut save = None;
        let mut restore = None;
        let mut save_cond = None;
        let mut restore_cond = None;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-domain" => {
                    domain = args.get(i + 1).cloned();
                    i += 2;
                }
                "-save_signal" => {
                    if let Some(spec) = args.get(i + 1) {
                        save = Some(self.parse_sig_sense_pair(spec)?);
                    }
                    i += 2;
                }
                "-restore_signal" => {
                    if let Some(spec) = args.get(i + 1) {
                        restore = Some(self.parse_sig_sense_pair(spec)?);
                    }
                    i += 2;
                }
                "-assert_s_mutex" | "-assert_r_mutex" | "-save_condition" => {
                    save_cond = args.get(i + 1).cloned();
                    i += 2;
                }
                "-restore_condition" => {
                    restore_cond = args.get(i + 1).cloned();
                    i += 2;
                }
                _ => i += 1,
            }
        }
        let ret = self
            .intent
            .retentions
            .entry(name.clone())
            .or_insert_with(|| UpfRetention {
                name: name.clone(),
                ..Default::default()
            });
        if let Some(d) = domain {
            ret.domain = d;
        }
        if let Some((s, sn)) = save {
            ret.save_signal = s;
            ret.save_sense = sn;
        }
        if let Some((s, sn)) = restore {
            ret.restore_signal = s;
            ret.restore_sense = sn;
        }
        if let Some(c) = save_cond {
            ret.save_condition = c;
        }
        if let Some(c) = restore_cond {
            ret.restore_condition = c;
        }
        Ok(name)
    }

    fn parse_sig_sense_pair(&self, spec: &str) -> Result<(String, SignalSense), SvccError> {
        let items = split_tcl_list(spec)?;
        let sig = items.first().map(|s| self.norm(s)).unwrap_or_default();
        let sense = items
            .get(1)
            .map(|s| parse_sense(s))
            .unwrap_or(SignalSense::High);
        Ok((sig, sense))
    }

    fn cmd_set_level_shifter(&mut self, args: &[String]) -> Result<String, SvccError> {
        let name = args
            .first()
            .ok_or_else(|| SvccError::new(STAGE, "set_level_shifter missing name"))?
            .clone();
        let mut ls = UpfLevelShifter {
            name: name.clone(),
            applies_to: AppliesTo::Both,
            rule: LsRule::Both,
            location: Location::Automatic,
            threshold_uv: 50_000,
            ..Default::default()
        };
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-domain" => {
                    ls.domain = args.get(i + 1).cloned().unwrap_or_default();
                    i += 2;
                }
                "-source" => {
                    if let Some(v) = args.get(i + 1) {
                        ls.source_supply_set = self.norm(v);
                    }
                    i += 2;
                }
                "-sink" => {
                    if let Some(v) = args.get(i + 1) {
                        ls.sink_supply_set = self.norm(v);
                    }
                    i += 2;
                }
                "-applies_to" => {
                    ls.applies_to =
                        parse_applies_to(args.get(i + 1).map(String::as_str).unwrap_or("both"));
                    i += 2;
                }
                "-rule" => {
                    ls.rule = match args.get(i + 1).map(String::as_str).unwrap_or("both") {
                        "low_to_high" => LsRule::LowToHigh,
                        "high_to_low" => LsRule::HighToLow,
                        _ => LsRule::Both,
                    };
                    i += 2;
                }
                "-location" => {
                    ls.location =
                        parse_location(args.get(i + 1).map(String::as_str).unwrap_or("automatic"));
                    i += 2;
                }
                "-threshold" => {
                    if let Some(v) = args.get(i + 1) {
                        ls.threshold_uv = parse_voltage_uv(v)?;
                    }
                    i += 2;
                }
                "-elements" => {
                    if let Some(list) = args.get(i + 1) {
                        for el in split_tcl_list(list)? {
                            ls.elements.push(self.norm(&el));
                        }
                    }
                    i += 2;
                }
                "-exclude_elements" => {
                    if let Some(list) = args.get(i + 1) {
                        for el in split_tcl_list(list)? {
                            ls.exclude_elements.push(self.norm(&el));
                        }
                    }
                    i += 2;
                }
                _ => i += 1,
            }
        }
        self.intent.level_shifters.insert(name.clone(), ls);
        Ok(name)
    }

    fn cmd_set_port_attributes(&mut self, args: &[String]) -> Result<String, SvccError> {
        let mut pa = UpfPortAttribute {
            applies_to: AppliesTo::Both,
            ..Default::default()
        };
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-ports" => {
                    if let Some(list) = args.get(i + 1) {
                        for p in split_tcl_list(list)? {
                            pa.ports.push(self.norm(&p));
                        }
                    }
                    i += 2;
                }
                "-domains" => {
                    if let Some(list) = args.get(i + 1) {
                        pa.domains.extend(split_tcl_list(list)?);
                    }
                    i += 2;
                }
                "-elements" => {
                    if let Some(list) = args.get(i + 1) {
                        for el in split_tcl_list(list)? {
                            pa.elements.push(self.norm(&el));
                        }
                    }
                    i += 2;
                }
                "-model" => {
                    pa.model = args.get(i + 1).cloned().unwrap_or_default();
                    i += 2;
                }
                "-applies_to" => {
                    pa.applies_to =
                        parse_applies_to(args.get(i + 1).map(String::as_str).unwrap_or("both"));
                    i += 2;
                }
                "-driver_supply" => {
                    if let Some(v) = args.get(i + 1) {
                        pa.driver_supply = self.norm(v);
                    }
                    i += 2;
                }
                "-receiver_supply" => {
                    if let Some(v) = args.get(i + 1) {
                        pa.receiver_supply = self.norm(v);
                    }
                    i += 2;
                }
                "-clamp_value" => {
                    if let Some(v) = args.get(i + 1) {
                        pa.clamp_value = Some(parse_clamp(v));
                    }
                    i += 2;
                }
                "-is_isolated" => {
                    if let Some(v) = args.get(i + 1) {
                        pa.is_isolated = Some(!matches!(
                            v.to_ascii_lowercase().as_str(),
                            "false" | "0" | "no"
                        ));
                    }
                    i += 2;
                }
                _ => i += 1,
            }
        }
        self.intent.port_attributes.push(pa);
        Ok(String::new())
    }
}

fn parse_voltage_uv(s: &str) -> Result<i32, SvccError> {
    let v: f64 = s
        .trim()
        .parse()
        .map_err(|_| SvccError::new(STAGE, format!("invalid voltage '{s}'")))?;
    Ok((v * 1_000_000.0).round() as i32)
}

fn parse_clamp(s: &str) -> ClampValue {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "one" => ClampValue::One,
        "latch" => ClampValue::Latch,
        "z" | "highz" => ClampValue::HighZ,
        _ => ClampValue::Zero,
    }
}

fn parse_sense(s: &str) -> SignalSense {
    match s.trim().to_ascii_lowercase().as_str() {
        "low" | "0" => SignalSense::Low,
        "posedge" => SignalSense::Posedge,
        "negedge" => SignalSense::Negedge,
        _ => SignalSense::High,
    }
}

fn parse_applies_to(s: &str) -> AppliesTo {
    match s.trim().to_ascii_lowercase().as_str() {
        "inputs" => AppliesTo::Inputs,
        "both" => AppliesTo::Both,
        _ => AppliesTo::Outputs,
    }
}

fn parse_location(s: &str) -> Location {
    match s.trim().to_ascii_lowercase().as_str() {
        "parent" => Location::Parent,
        "fanout" => Location::Fanout,
        "automatic" => Location::Automatic,
        _ => Location::SelfLoc,
    }
}

fn parse_simstate(s: &str) -> SimState {
    match s.trim().to_ascii_uppercase().as_str() {
        "CORRUPT" | "NOT_NORMAL" => SimState::Corrupt,
        "CORRUPT_ON_ACTIVITY" => SimState::CorruptOnActivity,
        "CORRUPT_STATE_ON_CHANGE" => SimState::CorruptStateOnChange,
        _ => SimState::Normal,
    }
}

fn parse_power_state_entry(spec: &str) -> Result<UpfPowerStateEntry, SvccError> {
    let items = split_tcl_list(spec)?;
    let name = items
        .first()
        .ok_or_else(|| SvccError::new(STAGE, "add_power_state -state empty"))?
        .clone();
    let mut e = UpfPowerStateEntry {
        name,
        simstate: SimState::Normal,
        ..Default::default()
    };
    let mut i = 1;
    while i < items.len() {
        match items[i].as_str() {
            "-supply_expr" => {
                e.supply_expr = items.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "-logic_expr" => {
                e.logic_expr = items.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "-simstate" => {
                e.simstate =
                    parse_simstate(items.get(i + 1).map(String::as_str).unwrap_or("NORMAL"));
                e.has_explicit_simstate = true;
                i += 2;
            }
            "-illegal" => {
                e.illegal = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
    Ok(e)
}

#[derive(Clone, Debug)]
enum TclWord {
    Bare(String),
    Braced(String),
    Quoted(String),
}

fn slice_balanced(
    chars: &[char],
    start_idx: usize,
    open: char,
    close: char,
) -> Result<(String, usize), SvccError> {
    let mut depth = 0usize;
    let mut i = start_idx;
    let mut out = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            if depth > 0 {
                out.push(c);
                out.push(chars[i + 1]);
            }
            i += 2;
            continue;
        }
        if c == open {
            if depth > 0 {
                out.push(c);
            }
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Ok((out, i + 1));
            }
            out.push(c);
        } else if depth > 0 {
            out.push(c);
        }
        i += 1;
    }
    Err(SvccError::new(
        STAGE,
        format!("unterminated '{open}...{close}' in UPF script"),
    ))
}

fn split_tcl_statements(src: &str) -> Result<Vec<Vec<TclWord>>, SvccError> {
    // Handle `\<newline>` continuations first (outside braces, Tcl also treats `\<newline>`
    // plus leading horizontal whitespace as a single space).
    let mut joined = String::with_capacity(src.len());
    let raw_chars: Vec<char> = src.chars().collect();
    let mut k = 0;
    let mut brace_depth = 0usize;
    while k < raw_chars.len() {
        if raw_chars[k] == '{' {
            brace_depth += 1;
            joined.push('{');
            k += 1;
        } else if raw_chars[k] == '}' {
            brace_depth = brace_depth.saturating_sub(1);
            joined.push('}');
            k += 1;
        } else if brace_depth == 0 && raw_chars[k] == '\\' {
            if raw_chars.get(k + 1) == Some(&'\n') {
                k += 2;
                while k < raw_chars.len() && (raw_chars[k] == ' ' || raw_chars[k] == '\t') {
                    k += 1;
                }
                joined.push(' ');
                continue;
            } else if raw_chars.get(k + 1) == Some(&'\r') && raw_chars.get(k + 2) == Some(&'\n') {
                k += 3;
                while k < raw_chars.len() && (raw_chars[k] == ' ' || raw_chars[k] == '\t') {
                    k += 1;
                }
                joined.push(' ');
                continue;
            }
            joined.push('\\');
            k += 1;
        } else {
            joined.push(raw_chars[k]);
            k += 1;
        }
    }

    let chars: Vec<char> = joined.chars().collect();
    let mut stmts = Vec::new();
    let mut cur = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == ' ' || c == '\t' || c == '\r' {
            i += 1;
            continue;
        }
        if c == '\n' || c == ';' {
            if !cur.is_empty() {
                stmts.push(std::mem::take(&mut cur));
            }
            i += 1;
            continue;
        }
        if c == '#' && cur.is_empty() {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '{' {
            let (inner, next_i) = slice_balanced(&chars, i, '{', '}')?;
            cur.push(TclWord::Braced(inner));
            i = next_i;
            continue;
        }
        if c == '"' {
            i += 1;
            let mut s = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    s.push(chars[i]);
                    s.push(chars[i + 1]);
                    i += 2;
                } else if chars[i] == '[' {
                    let (sub, next_i) = slice_balanced(&chars, i, '[', ']')?;
                    s.push('[');
                    s.push_str(&sub);
                    s.push(']');
                    i = next_i;
                } else {
                    s.push(chars[i]);
                    i += 1;
                }
            }
            if i >= chars.len() {
                return Err(SvccError::new(STAGE, "unterminated '\"' in UPF script"));
            }
            i += 1;
            cur.push(TclWord::Quoted(s));
            continue;
        }
        let mut s = String::new();
        while i < chars.len() {
            let ch = chars[i];
            if ch.is_whitespace() || ch == ';' {
                break;
            }
            if ch == '[' {
                let (sub, next_i) = slice_balanced(&chars, i, '[', ']')?;
                s.push('[');
                s.push_str(&sub);
                s.push(']');
                i = next_i;
            } else if ch == '\\' && i + 1 < chars.len() {
                s.push(ch);
                s.push(chars[i + 1]);
                i += 2;
            } else {
                s.push(ch);
                i += 1;
            }
        }
        cur.push(TclWord::Bare(s));
    }
    if !cur.is_empty() {
        stmts.push(cur);
    }
    Ok(stmts)
}

/// Splits a Tcl list string (`{a b} c "d e"`) into elements without variable expansion.
pub fn split_tcl_list(s: &str) -> Result<Vec<String>, SvccError> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        if chars[i] == '{' {
            let (inner, next_i) = slice_balanced(&chars, i, '{', '}')?;
            out.push(inner);
            i = next_i;
        } else if chars[i] == '"' {
            i += 1;
            let mut w = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                w.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            out.push(w);
        } else {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        }
    }
    Ok(out)
}

// ============================================================================
// UPF Boolean Control Expression AST (`!en`, `a && !b || c`)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpfBoolExpr {
    Const(bool),
    Port(String),
    Not(Box<UpfBoolExpr>),
    And(Box<UpfBoolExpr>, Box<UpfBoolExpr>),
    Or(Box<UpfBoolExpr>, Box<UpfBoolExpr>),
    Xor(Box<UpfBoolExpr>, Box<UpfBoolExpr>),
}

pub fn parse_upf_bool_expr(src: &str) -> Result<UpfBoolExpr, SvccError> {
    let toks = lex_bool(src)?;
    if toks.is_empty() {
        return Ok(UpfBoolExpr::Const(true));
    }
    let mut pos = 0usize;
    let e = parse_or(&toks, &mut pos)?;
    if pos != toks.len() {
        return Err(SvccError::new(
            STAGE,
            format!(
                "unexpected token '{}' in UPF boolean expression '{src}'",
                toks[pos]
            ),
        ));
    }
    Ok(e)
}

fn lex_bool(src: &str) -> Result<Vec<String>, SvccError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '&' && chars.get(i + 1) == Some(&'&') {
            out.push("&&".into());
            i += 2;
        } else if c == '|' && chars.get(i + 1) == Some(&'|') {
            out.push("||".into());
            i += 2;
        } else if "==!=".contains(c) && chars.get(i + 1) == Some(&'=') {
            out.push(chars[i..i + 2].iter().collect());
            i += 2;
        } else if "()!&|^~".contains(c) {
            out.push(c.to_string());
            i += 1;
        } else if c.is_ascii_alphanumeric() || c == '_' || c == '/' || c == '.' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '_'
                    || chars[i] == '/'
                    || chars[i] == '.')
            {
                i += 1;
            }
            out.push(chars[start..i].iter().collect());
        } else {
            return Err(SvccError::new(
                STAGE,
                format!("unsupported character '{c}' in UPF boolean expression '{src}'"),
            ));
        }
    }
    Ok(out)
}

fn parse_or(toks: &[String], pos: &mut usize) -> Result<UpfBoolExpr, SvccError> {
    let mut lhs = parse_xor(toks, pos)?;
    while matches!(toks.get(*pos).map(String::as_str), Some("||" | "|")) {
        *pos += 1;
        let rhs = parse_xor(toks, pos)?;
        lhs = UpfBoolExpr::Or(Box::new(lhs), Box::new(rhs));
    }
    Ok(lhs)
}

fn parse_xor(toks: &[String], pos: &mut usize) -> Result<UpfBoolExpr, SvccError> {
    let mut lhs = parse_and(toks, pos)?;
    while matches!(toks.get(*pos).map(String::as_str), Some("^" | "!=")) {
        *pos += 1;
        let rhs = parse_and(toks, pos)?;
        lhs = UpfBoolExpr::Xor(Box::new(lhs), Box::new(rhs));
    }
    Ok(lhs)
}

fn parse_and(toks: &[String], pos: &mut usize) -> Result<UpfBoolExpr, SvccError> {
    let mut lhs = parse_unary(toks, pos)?;
    while matches!(toks.get(*pos).map(String::as_str), Some("&&" | "&")) {
        *pos += 1;
        let rhs = parse_unary(toks, pos)?;
        lhs = UpfBoolExpr::And(Box::new(lhs), Box::new(rhs));
    }
    Ok(lhs)
}

fn parse_unary(toks: &[String], pos: &mut usize) -> Result<UpfBoolExpr, SvccError> {
    if matches!(toks.get(*pos).map(String::as_str), Some("!" | "~")) {
        *pos += 1;
        return Ok(UpfBoolExpr::Not(Box::new(parse_unary(toks, pos)?)));
    }
    if toks.get(*pos).map(String::as_str) == Some("(") {
        *pos += 1;
        let inner = parse_or(toks, pos)?;
        if toks.get(*pos).map(String::as_str) != Some(")") {
            return Err(SvccError::new(
                STAGE,
                "unclosed '(' in UPF boolean expression",
            ));
        }
        *pos += 1;
        return Ok(inner);
    }
    let Some(tok) = toks.get(*pos) else {
        return Err(SvccError::new(
            STAGE,
            "unexpected end of UPF boolean expression",
        ));
    };
    *pos += 1;
    let base = match tok.to_ascii_lowercase().as_str() {
        "1" | "true" | "1'b1" => UpfBoolExpr::Const(true),
        "0" | "false" | "1'b0" => UpfBoolExpr::Const(false),
        _ => UpfBoolExpr::Port(tok.clone()),
    };
    if toks.get(*pos).map(String::as_str) == Some("==") {
        *pos += 1;
        let rhs = parse_unary(toks, pos)?;
        return Ok(UpfBoolExpr::Not(Box::new(UpfBoolExpr::Xor(
            Box::new(base),
            Box::new(rhs),
        ))));
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apb_subsys_and_find_objects_upf() {
        let objs = vec![
            UpfDesignObject {
                path: "u_core".into(),
                kind: UpfObjectKind::Inst,
            },
            UpfDesignObject {
                path: "u_core.cfg_alpha_q".into(),
                kind: UpfObjectKind::Reg,
            },
            UpfDesignObject {
                path: "u_core.cfg_beta_q".into(),
                kind: UpfObjectKind::Reg,
            },
            UpfDesignObject {
                path: "u_core.acc_q".into(),
                kind: UpfObjectKind::Reg,
            },
        ];
        let upf = r#"
            upf_version 3.1
            set_design_top top_dut
            create_supply_port VDD_AON -direction in
            create_supply_port VSS -direction in
            add_port_state VDD_AON -state {ON_0V9 0.90} -state {LV_0V75 0.75}
            add_port_state VSS -state {GND 0.0}
            create_supply_set SS_AON -function {power VDD_AON} -function {ground VSS}
            create_supply_set SS_CORE -function {power VDD_CORE} -function {ground VSS}
            create_power_domain PD_AON -include_scope -supply {primary SS_AON}
            create_power_domain PD_CORE -elements {u_core} -supply {primary SS_CORE}
            create_power_switch SW_CORE \
                -domain PD_CORE \
                -input_supply_port {vin VDD_AON} \
                -output_supply_port {vout VDD_CORE} \
                -control_port {en u_pmu/pwr_en_q} \
                -on_state {ON vin {en && !override}}
            set cfgs [find_objects u_core -pattern {*cfg_*_q} -object_type reg]
            set_retention RET_CFG -domain PD_CORE \
                -retention_supply_set SS_AON \
                -elements $cfgs \
                -save_signal {u_pmu/save_q posedge} \
                -restore_signal {u_pmu/restore_q posedge}
        "#;
        let intent = parse_upf_str(upf, None, &objs).expect("parse upf");
        assert_eq!(intent.domains.len(), 2);
        assert_eq!(intent.find_domain_for_path("u_core.acc_q"), Some("PD_CORE"));
        assert_eq!(
            intent.find_domain_for_path("u_pmu.pwr_en_q"),
            Some("PD_AON")
        );
        let ret = &intent.retentions["RET_CFG"];
        assert_eq!(
            ret.elements,
            vec![
                "u_core.cfg_alpha_q".to_string(),
                "u_core.cfg_beta_q".to_string()
            ]
        );
        assert_eq!(ret.save_sense, SignalSense::Posedge);
    }
}
