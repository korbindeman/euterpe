//! `euterpe bench`: instruction counts per block on an emulated Cortex-M7,
//! plus checks of what each device's hot path compiles to.
//!
//! The pipeline:
//! 1. build `euterpe_bench_qemu` for `thumbv7em-none-eabihf` (release)
//! 2. build the `tools/qemu/euterpe_insn.c` plugin if needed
//! 3. run the firmware on QEMU's `mps2-an500` with the plugin, which counts
//!    instructions between the firmware's marker functions
//! 4. disassemble the firmware and walk the call graph from each device's
//!    measured call (`euterpe_bench::Hot<Graph>::process`, kept out of line
//!    so it always exists), looking for panics, soft float, f64 and allocation
//!
//! Instruction counts are deterministic, so they gate regressions exactly.
//! They are not cycles: the Cortex-M7 dual-issues, and memory placement and
//! caches matter on the real board. Until there is a calibration from the
//! board, cycles are estimated as one per instruction.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs};

use euterpe_core::device::{GRAPH_ARENA_BYTES, LARGE_GRAPH_ARENA_BYTES};
use euterpe_core::{BLOCK, Device};

use crate::json::Json;
use crate::registry::{self, Visitor};

const TARGET: &str = "thumbv7em-none-eabihf";
/// Peak stack allowed per device, building the graph included. On the Daisy
/// the stack shares the 128 KB DTCM with the graph arena (80 KB) and the
/// other statics.
const STACK_LIMIT_BYTES: u64 = 32 * 1024;
const FIRMWARE: &str = "euterpe_bench_qemu";
const QEMU_TIMEOUT: Duration = Duration::from_mins(2);
/// Instructions per cycle are not calibrated yet: assume one cycle each.
const CYCLES_PER_INSN: f64 = 1.0;

pub const HELP: &str = "\
    euterpe bench [<device>...] [options]
        Count instructions per block for each device on an emulated
        Cortex-M7 (QEMU mps2-an500), estimate cycles against the device's
        budget, and check its hot path for panics, soft float, f64 and
        allocation. With no device, benches every device.
        Needs qemu-system-arm, a C compiler with glib headers, and
        llvm-objdump (`rustup component add llvm-tools`).

BENCH OPTIONS:
    --baseline <file>     Compare with a baseline and fail on regressions.
    --tolerance <pct>     Allowed growth over the baseline [default: 0]
    --write-baseline <file>
                          Write the results as the new baseline.
";

pub struct Options {
    devices: Vec<String>,
    baseline: Option<PathBuf>,
    tolerance_pct: f64,
    write_baseline: Option<PathBuf>,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut opts = Options {
            devices: Vec::new(),
            baseline: None,
            tolerance_pct: 0.0,
            write_baseline: None,
        };
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            let mut value = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| format!("{arg} needs a value"))
            };
            match arg.as_str() {
                "--baseline" => opts.baseline = Some(value()?.into()),
                "--write-baseline" => opts.write_baseline = Some(value()?.into()),
                "--tolerance" => {
                    let v = value()?;
                    opts.tolerance_pct = v
                        .trim_end_matches('%')
                        .parse()
                        .ok()
                        .filter(|t: &f64| t.is_finite() && *t >= 0.0)
                        .ok_or_else(|| format!("--tolerance takes a percentage >= 0, got `{v}`"))?;
                }
                flag if flag.starts_with("--") => {
                    return Err(format!(
                        "unknown option `{flag}`; valid: --baseline, --tolerance, --write-baseline"
                    ));
                }
                device => {
                    registry::visit(device, Budget).ok_or_else(|| crate::unknown_device(device))?;
                    opts.devices.push(device.to_string());
                }
            }
        }
        Ok(opts)
    }
}

/// `(budget cycles per block, graph type name)` of a device.
struct Budget;

impl Visitor for Budget {
    type Out = (u32, &'static str);
    fn visit<D: Device>(self) -> Self::Out {
        (
            D::BUDGET_CYCLES_PER_BLOCK,
            std::any::type_name::<D::Graph>(),
        )
    }
}

/// One measured scenario.
struct Measurement {
    device: String,
    scenario: String,
    insns_per_block: f64,
}

pub fn run(opts: &Options) -> Result<(Json, bool), String> {
    let root = workspace_root()?;
    let elf = build_firmware(&root)?;
    let objdump = find_objdump()?;
    let plugin = build_plugin(&root)?;

    let symbols = symbols(&objdump, &elf)?;
    let QemuRun {
        measurements,
        stack,
        graph_bytes,
    } = run_qemu(&elf, &plugin, &symbols)?;
    let disasm = Disassembly::parse(&tool_output(
        Command::new(&objdump)
            .args(["-d", "-C", "--no-show-raw-insn", "--mcpu=cortex-m7"])
            .arg(&elf),
    )?);

    let wanted = |d: &str| opts.devices.is_empty() || opts.devices.iter().any(|w| w == d);
    let baseline = match &opts.baseline {
        Some(path) => Some(read_baseline(path)?),
        None => None,
    };

    let mut pass = true;
    let mut devices = Vec::new();
    for (name, _) in registry::DEVICES {
        if !wanted(name) {
            continue;
        }
        let (budget, graph) = registry::visit(name, Budget).expect("registered device");
        let mut scenarios = Vec::new();
        let mut worst: f64 = 0.0;
        for m in measurements.iter().filter(|m| m.device == *name) {
            let cycles = m.insns_per_block * CYCLES_PER_INSN;
            worst = worst.max(cycles);
            let (fields, regressed) = scenario_report(m, budget, baseline.as_ref(), opts);
            pass &= !regressed;
            scenarios.push(fields);
        }
        if scenarios.is_empty() {
            return Err(format!(
                "`{name}` has no bench results; the bench firmware is out of date or failed"
            ));
        }
        let over_budget = worst > f64::from(budget);
        pass &= !over_budget;
        let hot = disasm.hot_path(graph);
        pass &= hot.ok();
        let stack_bytes = stack.get(*name).copied();
        let stack_over = stack_bytes.is_none_or(|b| b > STACK_LIMIT_BYTES);
        pass &= !stack_over;
        let graph = graph_bytes.get(*name).copied();
        let memory = graph.and_then(graph_memory);
        pass &= memory.is_some();
        devices.push(Json::Obj(vec![
            ("device", Json::str(*name)),
            ("budget_cycles_per_block", Json::num(budget)),
            ("over_budget", Json::Bool(over_budget)),
            (
                "stack_bytes",
                stack_bytes.map_or(Json::Null, |b| Json::num(b as f64)),
            ),
            ("stack_over_limit", Json::Bool(stack_over)),
            (
                "graph_bytes",
                graph.map_or(Json::Null, |b| Json::num(b as f64)),
            ),
            ("graph_memory", memory.map_or(Json::Null, Json::str)),
            ("scenarios", Json::Arr(scenarios)),
            ("hot_path", hot.to_json()),
        ]));
    }

    if let Some(path) = &opts.write_baseline {
        write_baseline(path, &measurements)?;
    }

    let report = Json::Obj(vec![
        ("pass", Json::Bool(pass)),
        (
            "target",
            Json::str("qemu mps2-an500 (Cortex-M7), thumbv7em-none-eabihf, target-cpu=cortex-m7"),
        ),
        (
            "metric",
            Json::str("guest instructions per 32-sample block, probe overhead removed"),
        ),
        (
            "cycle_estimate",
            Json::str("1 cycle per instruction: not calibrated against the board yet"),
        ),
        ("blocks_per_scenario", Json::num(euterpe_bench::BLOCKS)),
        ("stack_limit_bytes", Json::num(STACK_LIMIT_BYTES as f64)),
        (
            "graph_arena_bytes",
            Json::Obj(vec![
                ("dtcm", Json::num(GRAPH_ARENA_BYTES as f64)),
                ("axi_sram", Json::num(LARGE_GRAPH_ARENA_BYTES as f64)),
            ]),
        ),
        ("firmware", section_sizes(&objdump, &elf)?),
        ("devices", Json::Arr(devices)),
    ]);
    Ok((report, pass))
}

/// One scenario's JSON, and whether it regressed against the baseline.
fn scenario_report(
    m: &Measurement,
    budget: u32,
    baseline: Option<&Baseline>,
    opts: &Options,
) -> (Json, bool) {
    let cycles = m.insns_per_block * CYCLES_PER_INSN;
    let mut regressed = false;
    let mut fields = vec![
        ("scenario", Json::str(&m.scenario)),
        ("insns_per_block", Json::num(round1(m.insns_per_block))),
        (
            "insns_per_sample",
            Json::num(round1(m.insns_per_block / BLOCK as f64)),
        ),
        ("est_cycles_per_block", Json::num(cycles.round())),
        (
            "budget_pct",
            Json::num(round1(100.0 * cycles / f64::from(budget))),
        ),
    ];
    if let Some(base) = baseline {
        match base.get(&(m.device.clone(), m.scenario.clone())) {
            Some(&before) => {
                let change = 100.0 * (m.insns_per_block - before) / before;
                regressed = change > opts.tolerance_pct;
                fields.push(("baseline_insns_per_block", Json::num(before)));
                fields.push(("change_pct", Json::num(round1(change))));
                fields.push(("regressed", Json::Bool(regressed)));
            }
            None => fields.push(("baseline_insns_per_block", Json::Null)),
        }
    }
    (Json::Obj(fields), regressed)
}

/// Where the Daisy puts a graph of `bytes`: DTCM if it fits, else AXI SRAM,
/// else nowhere (it doesn't build).
fn graph_memory(bytes: u64) -> Option<&'static str> {
    let fits = |arena: usize| u64::try_from(arena).is_ok_and(|a| bytes <= a);
    if fits(GRAPH_ARENA_BYTES) {
        Some("dtcm")
    } else if fits(LARGE_GRAPH_ARENA_BYTES) {
        Some("axi_sram")
    } else {
        None
    }
}

fn round1(x: f64) -> f64 {
    // `+ 0.0` turns -0.0 into 0.0.
    (x * 10.0).round() / 10.0 + 0.0
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

fn workspace_root() -> Result<PathBuf, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize()
        .map_err(|e| format!("finding the workspace at {}: {e}", root.display()))
}

fn tool_output(cmd: &mut Command) -> Result<String, String> {
    let out = cmd
        .output()
        .map_err(|e| format!("running {}: {e}", cmd.get_program().display()))?;
    if !out.status.success() {
        return Err(format!(
            "{} failed: {}",
            cmd.get_program().display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn build_firmware(root: &Path) -> Result<PathBuf, String> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(root)
        .args([
            "build",
            "-q",
            "--release",
            "-p",
            FIRMWARE,
            "--target",
            TARGET,
        ])
        .stdout(Stdio::null())
        .status()
        .map_err(|e| format!("running cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "building {FIRMWARE} failed (is the target installed? `rustup target add {TARGET}`)"
        ));
    }
    let target_dir =
        env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    Ok(target_dir.join(TARGET).join("release").join(FIRMWARE))
}

fn find_objdump() -> Result<PathBuf, String> {
    if let Some(p) = env::var_os("EUTERPE_OBJDUMP") {
        return Ok(p.into());
    }
    // rustup's llvm-tools component.
    if let Ok(out) = Command::new("rustc").args(["--print", "sysroot"]).output() {
        let sysroot = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        if let Ok(entries) = fs::read_dir(sysroot.join("lib/rustlib")) {
            for entry in entries.flatten() {
                let p = entry.path().join("bin/llvm-objdump");
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
    }
    let mut names = vec!["llvm-objdump".to_string()];
    names.extend((14..=22).rev().map(|v| format!("llvm-objdump-{v}")));
    for name in names {
        if Command::new(&name)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return Ok(name.into());
        }
    }
    Err(
        "llvm-objdump not found; run `rustup component add llvm-tools` or set EUTERPE_OBJDUMP"
            .into(),
    )
}

fn build_plugin(root: &Path) -> Result<PathBuf, String> {
    let source = root.join("tools/qemu/euterpe_insn.c");
    let out = root.join("target/euterpe-qemu/euterpe_insn.so");
    let modified = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    let fresh = matches!((modified(&out), modified(&source)), (Some(o), Some(s)) if o >= s);
    if !fresh {
        let status = Command::new("sh")
            .arg(root.join("tools/qemu/build-plugin.sh"))
            .arg(&out)
            .stdout(Stdio::null())
            .status()
            .map_err(|e| format!("building the QEMU plugin: {e}"))?;
        if !status.success() {
            return Err(
                "building the QEMU plugin failed; see tools/qemu/build-plugin.sh for what it needs"
                    .into(),
            );
        }
    }
    Ok(out)
}

/// Addresses of the firmware's global symbols.
fn symbols(objdump: &Path, elf: &Path) -> Result<HashMap<String, u64>, String> {
    let text = tool_output(Command::new(objdump).arg("-t").arg(elf))?;
    // Lines look like: `000017c0 g     F .text  00000010 euterpe_bench_start`
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let addr = u64::from_str_radix(parts.next()?, 16).ok()?;
            let name = parts.next_back()?;
            Some((name.to_string(), addr))
        })
        .collect())
}

fn section_sizes(objdump: &Path, elf: &Path) -> Result<Json, String> {
    let text = tool_output(Command::new(objdump).arg("-h").arg(elf))?;
    // Lines look like: `  1 .text  00007d8c 00000400 TEXT`
    let mut fields = Vec::new();
    for (section, key) in [
        (".text", "text_bytes"),
        (".rodata", "rodata_bytes"),
        (".data", "data_bytes"),
        (".bss", "bss_bytes"),
    ] {
        let size = text.lines().find_map(|line| {
            let mut parts = line.split_whitespace().skip(1);
            (parts.next()? == section).then(|| u64::from_str_radix(parts.next()?, 16).ok())?
        });
        #[expect(
            clippy::cast_precision_loss,
            reason = "section sizes are far below 2^52"
        )]
        fields.push((key, size.map_or(Json::Null, |s| Json::num(s as f64))));
    }
    Ok(Json::Obj(fields))
}

fn run_qemu(elf: &Path, plugin: &Path, symbols: &HashMap<String, u64>) -> Result<QemuRun, String> {
    let addr = |name: &str| {
        symbols
            .get(name)
            .map(|a| format!("{a:#x}"))
            .ok_or_else(|| format!("marker `{name}` missing from the bench firmware"))
    };
    let plugin_arg = format!(
        "{},start={},stop={},end={}",
        plugin.display(),
        addr("euterpe_bench_start")?,
        addr("euterpe_bench_stop")?,
        addr("euterpe_bench_end")?
    );
    let mut child = Command::new("qemu-system-arm")
        .args([
            "-M",
            "mps2-an500",
            "-nographic",
            "-monitor",
            "none",
            "-serial",
            "none",
        ])
        .args(["-semihosting-config", "enable=on,target=native"])
        .arg("-kernel")
        .arg(elf)
        .args(["-plugin", &plugin_arg, "-d", "plugin"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("running qemu-system-arm (is QEMU installed?): {e}"))?;

    let started = Instant::now();
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if started.elapsed() > QEMU_TIMEOUT {
            let _ = child.kill();
            return Err(format!(
                "QEMU did not finish within {}s",
                QEMU_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(format!(
            "bench firmware failed under QEMU:\n{}\n{}",
            stdout.trim(),
            stderr.trim()
        ));
    }
    Ok(QemuRun {
        measurements: parse_qemu_output(&stdout, &stderr)?,
        stack: parse_tagged(&stdout, "stack "),
        graph_bytes: parse_tagged(&stdout, "graph "),
    })
}

/// What the bench firmware reported.
struct QemuRun {
    measurements: Vec<Measurement>,
    /// Stack high-water mark per device, in bytes.
    stack: HashMap<String, u64>,
    /// `size_of` the graph per device, in bytes.
    graph_bytes: HashMap<String, u64>,
}

/// The firmware's `<tag><device> <bytes>` lines (`stack `, `graph `).
fn parse_tagged(stdout: &str, tag: &str) -> HashMap<String, u64> {
    stdout
        .lines()
        .filter_map(|l| {
            let mut p = l.strip_prefix(tag)?.split_whitespace();
            Some((p.next()?.into(), p.next()?.parse().ok()?))
        })
        .collect()
}

/// Zip the firmware's `bench <device> <scenario> <blocks>` lines with the
/// plugin's `euterpe-bench <insns> <regions>` lines, and remove the probe
/// overhead measured by the first (`overhead`) scenario.
fn parse_qemu_output(stdout: &str, stderr: &str) -> Result<Vec<Measurement>, String> {
    let labels: Vec<(String, String, u64)> = stdout
        .lines()
        .filter_map(|l| {
            let mut p = l.strip_prefix("bench ")?.split_whitespace();
            Some((p.next()?.into(), p.next()?.into(), p.next()?.parse().ok()?))
        })
        .collect();
    let counts: Vec<(u64, u64)> = stderr
        .lines()
        .filter_map(|l| {
            let mut p = l.strip_prefix("euterpe-bench ")?.split_whitespace();
            Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?))
        })
        .collect();
    if labels.len() != counts.len() || labels.is_empty() {
        return Err(format!(
            "QEMU output doesn't match: {} scenarios, {} counts (does this QEMU support plugins?)",
            labels.len(),
            counts.len()
        ));
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "instruction counts are far below 2^52"
    )]
    let per_region = |(total, regions): (u64, u64)| total as f64 / regions.max(1) as f64;
    let mut overhead = 0.0;
    let mut out = Vec::new();
    for ((device, scenario, blocks), count) in labels.into_iter().zip(counts) {
        if count.1 != blocks {
            return Err(format!(
                "{device} {scenario}: firmware ran {blocks} blocks, plugin saw {} regions",
                count.1
            ));
        }
        if device == "overhead" {
            overhead = per_region(count);
            continue;
        }
        out.push(Measurement {
            device,
            scenario,
            insns_per_block: (per_region(count) - overhead).max(0.0),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Baselines: a text file, one `<device> <scenario> <insns_per_block>` per line
// ---------------------------------------------------------------------------

type Baseline = BTreeMap<(String, String), f64>;

fn read_baseline(path: &Path) -> Result<Baseline, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut map = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        let [device, scenario, insns] = parts[..] else {
            return Err(format!(
                "{}:{}: expected `<device> <scenario> <insns>`",
                path.display(),
                i + 1
            ));
        };
        let insns: f64 = insns
            .parse()
            .map_err(|_| format!("{}:{}: `{insns}` is not a number", path.display(), i + 1))?;
        map.insert((device.to_string(), scenario.to_string()), insns);
    }
    Ok(map)
}

fn write_baseline(path: &Path, measurements: &[Measurement]) -> Result<(), String> {
    let mut text = String::from(
        "# euterpe bench baseline: guest instructions per block on qemu mps2-an500.\n\
         # Regenerate with `just bench-baseline` after an intended change.\n",
    );
    for m in measurements {
        let _ = writeln!(
            text,
            "{} {} {}",
            m.device,
            m.scenario,
            // Exact: a total over 64 blocks has a short, exact decimal
            // form, and the comparison has zero tolerance by default.
            m.insns_per_block
        );
    }
    fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Hot-path checks over the disassembly
// ---------------------------------------------------------------------------

struct Function {
    bytes: u64,
    calls: BTreeSet<String>,
    f64_insns: usize,
}

struct Disassembly {
    functions: HashMap<String, Function>,
}

impl Disassembly {
    /// Parse `llvm-objdump -d -C` output.
    fn parse(text: &str) -> Self {
        let mut functions = HashMap::new();
        let mut current: Option<(String, u64, u64, Function)> = None; // name, start, last addr
        let finish = |cur: Option<(String, u64, u64, Function)>,
                      functions: &mut HashMap<String, Function>| {
            if let Some((name, start, last, mut f)) = cur {
                f.bytes = last.saturating_sub(start) + 2;
                functions.insert(name, f);
            }
        };
        for line in text.lines() {
            // Function header: `00001330 <name>:`
            if let Some((addr, rest)) = line.split_once(" <")
                && let Some(name) = rest.strip_suffix(">:")
                && let Ok(start) = u64::from_str_radix(addr.trim(), 16)
            {
                finish(current.take(), &mut functions);
                current = Some((
                    name.to_string(),
                    start,
                    start,
                    Function {
                        bytes: 0,
                        calls: BTreeSet::new(),
                        f64_insns: 0,
                    },
                ));
                continue;
            }
            let Some((_, _, last, f)) = current.as_mut() else {
                continue;
            };
            // Instruction: `    1338:      	vmul.f32	s0, s0, s5`
            let Some((addr, insn)) = line.trim_start().split_once(':') else {
                continue;
            };
            let Ok(addr) = u64::from_str_radix(addr, 16) else {
                continue;
            };
            *last = addr;
            let insn = insn.trim();
            let mnemonic = insn.split_whitespace().next().unwrap_or("");
            if mnemonic.contains(".f64") {
                f.f64_insns += 1;
            }
            // Calls and tail calls name their target: `bl 0x1e20 <__pre_init>`.
            // Targets with an offset (`<f+0x10>`) are branches within a function.
            let is_branch = mnemonic == "bl"
                || mnemonic == "blx"
                || mnemonic.starts_with("b.")
                || mnemonic == "b";
            if is_branch
                && let Some(target) = branch_target(insn)
                && !target.contains("+0x")
            {
                f.calls.insert(target.to_string());
            }
        }
        finish(current.take(), &mut functions);
        Disassembly { functions }
    }

    fn hot_path(&self, graph: &str) -> HotPath {
        let root = format!("<euterpe_bench::Hot<{graph}>>::process");
        let mut hot = HotPath {
            root: root.clone(),
            ..HotPath::default()
        };
        if !self.functions.contains_key(&root) {
            return hot;
        }
        hot.found = true;
        let mut seen = BTreeSet::from([root.clone()]);
        let mut queue = VecDeque::from([root]);
        while let Some(name) = queue.pop_front() {
            // A panic is reported, not followed: what the panic handler
            // calls is not part of the hot path.
            let followed = hot.classify(&name);
            let Some(f) = self.functions.get(&name).filter(|_| followed) else {
                continue;
            };
            hot.code_bytes += f.bytes;
            hot.f64_insns += f.f64_insns;
            for callee in &f.calls {
                if seen.insert(callee.clone()) {
                    queue.push_back(callee.clone());
                }
            }
        }
        hot.functions = seen.into_iter().collect();
        hot
    }
}

/// The symbol a branch names: `bl 0x1e20 <name> @ imm = #4` gives `name`.
/// Demangled names contain `<` and `>` themselves, so take everything from
/// the first ` <` to the last `>` before the comment.
fn branch_target(insn: &str) -> Option<&str> {
    let operands = insn.split_once(" @").map_or(insn, |(i, _)| i);
    let start = operands.find(" <")? + 2;
    let end = operands.rfind('>')?;
    operands.get(start..end)
}

#[derive(Default)]
struct HotPath {
    root: String,
    found: bool,
    functions: Vec<String>,
    code_bytes: u64,
    f64_insns: usize,
    panics: Vec<String>,
    soft_float: Vec<String>,
    alloc: Vec<String>,
    slow_calls: Vec<String>,
}

impl HotPath {
    /// Record `name` if it is a problem. Returns whether to follow its calls.
    fn classify(&mut self, name: &str) -> bool {
        let is_panic = name.contains("core::panicking")
            || name.contains("core::slice::index::slice_")
            || name.contains("core::option::unwrap_failed")
            || name.contains("core::option::expect_failed")
            || name.contains("core::result::unwrap_failed")
            || name.contains("panic_bounds_check");
        let is_int_helper = [
            "mem", "uidiv", "idiv", "uldivmod", "ldivmod", "lmul", "llsl", "llsr", "lasr",
        ]
        .iter()
        .any(|p| name.starts_with(&format!("__aeabi_{p}")));
        let is_soft_float = (name.starts_with("__aeabi_") && !is_int_helper)
            || name.ends_with("df3")
            || name.ends_with("sf3")
            || name.ends_with("sfdf2")
            || name.ends_with("dfsf2")
            || name.starts_with("__fix")
            || name.starts_with("__float");
        if is_panic {
            self.panics.push(name.into());
            return false;
        } else if is_soft_float {
            self.soft_float.push(name.into());
        } else if name.contains("__rust_alloc")
            || name.contains("__rust_dealloc")
            || name.contains("__rg_alloc")
        {
            self.alloc.push(name.into());
        } else if name.contains("divmod")
            || name.contains("u64_div_rem")
            || name.starts_with("compiler_builtins::")
        {
            self.slow_calls.push(name.into());
        }
        true
    }

    fn ok(&self) -> bool {
        self.found
            && self.panics.is_empty()
            && self.soft_float.is_empty()
            && self.alloc.is_empty()
            && self.f64_insns == 0
    }

    fn to_json(&self) -> Json {
        let strs = |v: &[String]| Json::Arr(v.iter().map(Json::str).collect());
        if !self.found {
            return Json::Obj(vec![
                ("ok", Json::Bool(false)),
                ("root", Json::str(&self.root)),
                (
                    "note",
                    Json::str(
                        "the measured call is missing from the firmware, so nothing was checked",
                    ),
                ),
            ]);
        }
        #[expect(clippy::cast_precision_loss, reason = "small counts")]
        Json::Obj(vec![
            ("ok", Json::Bool(self.ok())),
            ("root", Json::str(&self.root)),
            ("code_bytes", Json::num(self.code_bytes as f64)),
            ("functions", strs(&self.functions)),
            ("panics", strs(&self.panics)),
            ("soft_float", strs(&self.soft_float)),
            ("f64_instructions", Json::num(self.f64_insns as f64)),
            ("alloc", strs(&self.alloc)),
            ("slow_calls", strs(&self.slow_calls)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISASM: &str = "
00001000 <<euterpe_bench::Hot<demo::Fast>>::process>:
    1000:      \tvmul.f32\ts0, s0, s1
    1004:      \tbl\t0x2000 <helper>     @ imm = #0xff8
    1008:      \tbne\t0x1000 <<euterpe_bench::Hot<demo::Fast>>::process+0x0>
    100a:      \tbx\tlr

00002000 <helper>:
    2000:      \tvadd.f32\ts0, s0, s1
    2004:      \tbx\tlr

00003000 <<euterpe_bench::Hot<demo::Slow>>::process>:
    3000:      \tvmul.f64\td0, d0, d1
    3004:      \tbl\t0x4000 <core::panicking::panic_bounds_check>
    3008:      \tb.w\t0x5000 <__aeabi_fmul>

00004000 <core::panicking::panic_bounds_check>:
    4000:      \tbx\tlr
";

    #[test]
    fn clean_hot_path_passes() {
        let d = Disassembly::parse(DISASM);
        let hot = d.hot_path("demo::Fast");
        assert!(hot.found && hot.ok(), "{:?}", hot.functions);
        assert_eq!(hot.functions.len(), 2);
        assert_eq!(hot.code_bytes, 12 + 6);
    }

    #[test]
    fn finds_panics_soft_float_and_f64() {
        let d = Disassembly::parse(DISASM);
        let hot = d.hot_path("demo::Slow");
        assert!(!hot.ok());
        assert_eq!(hot.f64_insns, 1);
        assert_eq!(hot.panics, ["core::panicking::panic_bounds_check"]);
        assert_eq!(hot.soft_float, ["__aeabi_fmul"]);
    }

    #[test]
    fn branch_targets_keep_demangled_generics() {
        assert_eq!(
            branch_target("bl\t0x10 <<a::B as c::D>::f> @ imm = #4"),
            Some("<a::B as c::D>::f")
        );
        assert_eq!(
            branch_target("bne\t0x10 <<a::B as c::D>::f+0x8>"),
            Some("<a::B as c::D>::f+0x8")
        );
        assert_eq!(branch_target("bx\tlr"), None);
    }

    #[test]
    fn missing_root_fails() {
        let hot = Disassembly::parse(DISASM).hot_path("demo::Inlined");
        assert!(!hot.found && !hot.ok());
    }

    #[test]
    fn qemu_output_is_zipped_and_overhead_removed() {
        let stdout = "bench overhead empty 4\nbench demo.gain static 4\n";
        let stderr = "euterpe-bench 32 4\neuterpe-bench 432 4\n";
        let m = parse_qemu_output(stdout, stderr).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(
            (m[0].device.as_str(), m[0].scenario.as_str()),
            ("demo.gain", "static")
        );
        assert!((m[0].insns_per_block - 100.0).abs() < 1e-9);
    }

    #[test]
    fn mismatched_qemu_output_is_an_error() {
        assert!(parse_qemu_output("bench a b 4\n", "").is_err());
        assert!(parse_qemu_output("bench a b 4\n", "euterpe-bench 10 3\n").is_err());
    }
}
