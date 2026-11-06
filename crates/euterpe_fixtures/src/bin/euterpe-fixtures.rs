//! `euterpe-fixtures`: fetch, verify and excerpt the guitar test fixtures.
//!
//! ```text
//! euterpe-fixtures list
//! euterpe-fixtures fetch [<dataset>...]     download, verify and unzip into fixtures/cache/
//! euterpe-fixtures verify [<dataset>...]    re-hash everything in the cache
//! euterpe-fixtures excerpt [<id>...]        cut committed clips from the cache
//! ```
//!
//! Downloads shell out to `curl` and `unzip`, so there is no HTTP, TLS or
//! zip dependency. Prints JSON, like the `euterpe` CLI.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use euterpe_fixtures::{
    ARCHIVES, Archive, DATASETS, EXCERPTS, Kind, cache_dir, cached, committed_dir, cut, manifest,
    sha256,
};

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn archive_path(a: &Archive) -> PathBuf {
    cache_dir().join(a.dataset.id).join(a.file)
}

/// `<file>.sha256` records a verified hash so `fetch` doesn't re-hash
/// gigabytes every run. `verify` ignores it.
fn marker_path(a: &Archive) -> PathBuf {
    cache_dir()
        .join(a.dataset.id)
        .join(format!("{}.sha256", a.file))
}

fn size_of(p: &Path) -> Option<u64> {
    fs::metadata(p).ok().map(|m| m.len())
}

fn hash_file(p: &Path) -> Result<String, String> {
    let f = fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    sha256::hash_reader(std::io::BufReader::new(f)).map_err(|e| format!("{}: {e}", p.display()))
}

fn verify_archive(a: &Archive, trust_marker: bool) -> Result<bool, String> {
    let p = archive_path(a);
    if size_of(&p) != Some(a.bytes) {
        return Ok(false);
    }
    if trust_marker && fs::read_to_string(marker_path(a)).is_ok_and(|m| m.trim() == a.sha256) {
        return Ok(true);
    }
    let h = hash_file(&p)?;
    if h != a.sha256 {
        return Err(format!(
            "{}: sha256 {h} does not match the manifest ({}). Upstream changed or the download is corrupt; delete it and fetch again.",
            p.display(),
            a.sha256
        ));
    }
    fs::write(marker_path(a), format!("{h}\n")).map_err(|e| e.to_string())?;
    Ok(true)
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("could not run {:?}: {e}", cmd.get_program()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed with {status}"))
    }
}

fn fetch_archive(a: &Archive) -> Result<&'static str, String> {
    let p = archive_path(a);
    fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut action = "cached";
    if !verify_archive(a, true)? {
        eprintln!("downloading {} ({:.0} MB)", a.file, a.bytes as f64 / 1e6);
        // Resumable, with retries: Zenodo is slow and drops connections.
        run(Command::new("curl")
            .args([
                "-L",
                "--fail",
                "--retry",
                "5",
                "--retry-delay",
                "5",
                "-C",
                "-",
                "-o",
            ])
            .arg(&p)
            .arg(a.url()))?;
        if !verify_archive(a, false)? {
            return Err(format!(
                "{}: size {:?} after download, expected {}",
                p.display(),
                size_of(&p),
                a.bytes
            ));
        }
        action = "downloaded";
    }
    if let Some(dir) = a.extract_dir() {
        let dest = cache_dir().join(a.dataset.id).join(dir);
        if !dest.is_dir() {
            eprintln!("unzipping {}", a.file);
            run(Command::new("unzip")
                .args(["-q", "-o"])
                .arg(&p)
                .arg("-d")
                .arg(&dest))?;
        }
    }
    Ok(action)
}

fn select_archives(datasets: &[String]) -> Result<Vec<&'static Archive>, String> {
    for d in datasets {
        if !DATASETS.iter().any(|x| x.id == d) {
            let ids: Vec<_> = DATASETS.iter().map(|x| x.id).collect();
            return Err(format!("unknown dataset `{d}`; valid: {}", ids.join(", ")));
        }
    }
    Ok(ARCHIVES
        .iter()
        .filter(|a| datasets.is_empty() || datasets.iter().any(|d| d == a.dataset.id))
        .collect())
}

fn cmd_fetch(args: &[String], trust_marker: bool) -> Result<String, String> {
    let mut rows = vec![];
    for a in select_archives(args)? {
        let state = if trust_marker {
            fetch_archive(a)?
        } else if verify_archive(a, false)? {
            "ok"
        } else {
            "missing"
        };
        rows.push(format!(
            "{{\"dataset\":{},\"file\":{},\"bytes\":{},\"state\":{}}}",
            json_str(a.dataset.id),
            json_str(a.file),
            a.bytes,
            json_str(state)
        ));
    }
    Ok(format!("[{}]", rows.join(",")))
}

fn cmd_excerpt(args: &[String]) -> Result<String, String> {
    let selected: Vec<_> = if args.is_empty() {
        EXCERPTS.iter().collect()
    } else {
        args.iter()
            .map(|id| {
                manifest::excerpt(id)
                    .ok_or_else(|| euterpe_fixtures::Error::UnknownFixture(id.clone()).to_string())
            })
            .collect::<Result<_, _>>()?
    };
    let mut rows = vec![];
    for e in selected {
        let r = cut(e).map_err(|err| {
            format!(
                "{}: {err}. Run `just fixtures fetch {}` first.",
                e.id, e.dataset.id
            )
        })?;
        let opt = |v: Option<String>| v.unwrap_or_else(|| "null".into());
        rows.push(format!(
            "{{\"id\":{},\"files\":[{}],\"start_s\":{},\"lag_samples\":{},\"inverted\":{},\"correlation\":{}}}",
            json_str(e.id),
            e.files().iter().map(|f| json_str(f)).collect::<Vec<_>>().join(","),
            r.start_s,
            opt(r.lag.map(|v| v.to_string())),
            r.inverted,
            opt(r.correlation.map(|v| format!("{v:.3}"))),
        ));
    }
    Ok(format!("[{}]", rows.join(",")))
}

fn cmd_list() -> String {
    let datasets: Vec<_> = DATASETS
        .iter()
        .map(|d| {
            format!(
                "{{\"id\":{},\"title\":{},\"license\":{},\"record\":{}}}",
                json_str(d.id),
                json_str(d.title),
                json_str(d.license),
                json_str(d.record_url)
            )
        })
        .collect();
    let archives: Vec<_> = ARCHIVES
        .iter()
        .map(|a| {
            format!(
                "{{\"dataset\":{},\"file\":{},\"bytes\":{},\"sha256\":{},\"present\":{}}}",
                json_str(a.dataset.id),
                json_str(a.file),
                a.bytes,
                json_str(a.sha256),
                size_of(&archive_path(a)) == Some(a.bytes)
            )
        })
        .collect();
    let excerpts: Vec<_> = EXCERPTS
        .iter()
        .map(|e| {
            let (kind, hardware, source_cached) = match e.kind {
                Kind::Di { di } => ("di", None, cached(&e.dataset, &di).is_some()),
                Kind::Paired {
                    di, out, hardware, ..
                } => (
                    "paired",
                    Some(hardware),
                    cached(&e.dataset, &di).is_some() && cached(&e.dataset, &out).is_some(),
                ),
            };
            let committed = e.files().iter().all(|f| committed_dir().join(f).is_file());
            format!(
                "{{\"id\":{},\"dataset\":{},\"kind\":{},\"hardware\":{},\"len_s\":{},\"committed\":{},\"source_cached\":{},\"note\":{}}}",
                json_str(e.id),
                json_str(e.dataset.id),
                json_str(kind),
                hardware.map_or("null".into(), json_str),
                e.len_s,
                committed,
                source_cached,
                json_str(e.note)
            )
        })
        .collect();
    format!(
        "{{\"cache\":{},\"committed\":{},\"datasets\":[{}],\"archives\":[{}],\"excerpts\":[{}]}}",
        json_str(&cache_dir().display().to_string()),
        json_str(&committed_dir().display().to_string()),
        datasets.join(","),
        archives.join(","),
        excerpts.join(",")
    )
}

const USAGE: &str = "usage: euterpe-fixtures <list|fetch|verify|excerpt> [args...]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match args.first().map(String::as_str) {
        Some("list") => Ok(cmd_list()),
        Some("fetch") => cmd_fetch(rest, true),
        Some("verify") => cmd_fetch(rest, false),
        Some("excerpt") => cmd_excerpt(rest),
        Some(other) => Err(format!("unknown command `{other}`; {USAGE}")),
        None => Err(USAGE.into()),
    };
    match result {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{{\"error\":{}}}", json_str(&e));
            ExitCode::FAILURE
        }
    }
}
