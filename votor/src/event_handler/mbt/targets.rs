//! Targeted traces: replays only the traces that reach a given scenario, each cut off at the
//! step where the scenario happens.
//!
//! A [`Target`] is a Quint expression over the spec's state that holds until the scenario
//! happens, such as `not(notarizedInWindow1)`. `quint run --invariant=<target>` samples traces
//! until it has the requested number of traces that violate it, and writes each of them up to
//! the first state where the scenario holds, with `#meta.status` set to `"violation"`. If it finds
//! fewer, the remaining files are ordinary traces, with status `"ok"`, which are not replayed:
//! the simulation tests already cover ordinary traces.
//!
//! quint-connect cannot pass `--invariant`, and would replay every written trace, so this module
//! runs `quint` itself and reads its ITF output with the `itf` crate. Each state's
//! `mbt::actionTaken` and `mbt::nondetPicks` become a [`SpecAction`], which the same driver as in
//! the simulation tests applies, and the driver's projection is compared with the state's
//! environment variable after every step, as quint-connect does.

use {
    super::{
        Instance, VotorMbtDriver,
        spec_types::{SpecAction, SpecState},
    },
    anyhow::{Context, anyhow, bail, ensure},
    itf::{Trace, Value, value::Record},
    quint_connect::Result,
    serde::Deserialize,
    std::{
        fs::File,
        io::BufReader,
        path::PathBuf,
        process::Command,
        time::{Duration, Instant},
    },
    tempfile::TempDir,
};

/// The vendored spec, which `quint` reads.
const SPEC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/event_handler/mbt/spec/statemachine.qnt"
);

/// A scenario to replay traces of, and how hard to look for them.
pub(super) struct Target {
    /// Used in messages, e.g. "notarized in window 1".
    pub(super) name: &'static str,
    /// A Quint expression that holds until the scenario happens, e.g.
    /// "not(notarizedInWindow1)".
    pub(super) invariant: &'static str,
    /// How many witnesses to ask for (`--n-traces`), within `max_samples` sampled traces.
    pub(super) witnesses: usize,
    /// How many traces to sample at most (`--max-samples`).
    pub(super) max_samples: usize,
    /// The length of each sampled trace (`--max-steps`).
    pub(super) max_steps: usize,
}

/// Runs `quint run --invariant` for `target` on the instance `I`, with `seed`, and returns the
/// traces that reach the target's scenario, in the order `quint` wrote them.
///
/// `quint` exits with 1 when it finds a violation, and with 0 when it finds none; in both cases
/// it writes `--n-traces` files. No witness gives an empty list. A failure that writes no trace
/// is an error that carries `quint`'s output.
fn witnesses<I: Instance>(target: &Target, seed: &str) -> Result<Vec<Trace<Value>>> {
    ensure!(
        target.witnesses <= target.max_samples,
        "target '{}' asks for {} witnesses in {} samples, but quint cannot write more traces than \
         it samples",
        target.name,
        target.witnesses,
        target.max_samples,
    );
    let dir = TempDir::with_prefix("votor-mbt-targets-")?;
    let output = Command::new("quint")
        .arg("run")
        .arg(SPEC)
        .args(["--main", I::MAIN, "--init", I::INIT, "--seed", seed])
        .args(["--invariant", target.invariant])
        .args(["--n-traces", &target.witnesses.to_string()])
        .args(["--max-samples", &target.max_samples.to_string()])
        .args(["--max-steps", &target.max_steps.to_string()])
        .arg("--mbt")
        .arg("--out-itf")
        .arg(dir.path().join("t_{seq}.itf.json"))
        .args(["--verbosity", "0"])
        .output()
        .context("cannot run `quint`; is it on PATH? (npm install -g @informalsystems/quint)")?;
    let quint_output = || {
        format!(
            "quint exited with {}:\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
    };

    // `quint` numbers the files `t_0`, `t_1`, ...: read them in that order, so that a witness's
    // index is the same for the same seed.
    let mut files: Vec<(usize, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir.path())? {
        let path = entry?.path();
        let seq = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("t_")?.strip_suffix(".itf.json"))
            .and_then(|seq| seq.parse().ok())
            .ok_or_else(|| anyhow!("unexpected file {} in quint's output", path.display()))?;
        files.push((seq, path));
    }
    files.sort();
    if files.is_empty() {
        bail!("quint wrote no trace; {}", quint_output());
    }

    let mut witnesses = Vec::new();
    for (_, path) in files {
        let file = File::open(&path)?;
        let trace: Trace<Value> = serde_json::from_reader(BufReader::new(file))
            .with_context(|| format!("cannot parse the ITF trace {}", path.display()))?;
        if trace.meta.other.get("status").map(String::as_str) == Some("violation") {
            witnesses.push(trace);
        }
    }
    if witnesses.is_empty() && !output.status.success() {
        bail!("quint failed without finding a witness; {}", quint_output());
    }
    Ok(witnesses)
}

/// Replays `trace` with `driver`: for every state, applies its action, then compares the state's
/// environment variable with the driver's projection.
fn replay_trace<I: Instance>(driver: &mut VotorMbtDriver<I>, trace: Trace<Value>) -> Result {
    for (step, state) in trace.states.into_iter().enumerate() {
        let Value::Record(mut state) = state.value else {
            bail!("step {step}: the ITF state is not a record");
        };
        let action_taken = take(&mut state, "mbt::actionTaken", step)?;
        let Value::String(action_taken) = action_taken else {
            bail!("step {step}: `mbt::actionTaken` is not a string: {action_taken:?}");
        };
        let nondet_picks = take(&mut state, "mbt::nondetPicks", step)?;
        let action = SpecAction::from_mbt(&action_taken, nondet_picks)
            .with_context(|| format!("step {step}"))?;
        let description = format!("step {step}, {action:?}");
        driver.apply(action).with_context(|| description.clone())?;

        let spec = SpecState::deserialize(value_at(state, I::STATE_PATH)?)
            .with_context(|| format!("{description}: cannot read the spec's state"))?;
        let agave = driver.project().context(description.clone())?;
        // `SpecState`'s `PartialEq` prints what differs.
        ensure!(
            spec == agave,
            "{description}: the state differs between the spec and agave (see the differences \
             printed above)"
        );
    }
    Ok(())
}

/// Removes the variable `name` from an ITF state.
fn take(state: &mut Record, name: &str, step: usize) -> Result<Value> {
    state
        .remove(name)
        .ok_or_else(|| anyhow!("step {step}: the ITF state has no `{name}`"))
}

/// The value at `path` in an ITF state.
fn value_at(state: Record, path: &[&str]) -> Result<Value> {
    let mut value = Value::Record(state);
    for name in path {
        let Value::Record(mut record) = value else {
            bail!("cannot read `{name}` of {path:?} from a value that is not a record");
        };
        value = record
            .remove(name)
            .ok_or_else(|| anyhow!("the ITF state has no `{name}` of {path:?}"))?;
    }
    Ok(value)
}

/// For each target, generates its witnesses with `quint run --invariant`, and replays them all
/// with one driver. Fails if a target has no witness at all; fewer witnesses than asked for are
/// only reported, because with a random seed and a low hit rate that happens by chance.
///
/// The seed comes from `QUINT_SEED`, or is random, as in the simulation tests. A failure names
/// the target, the witness, the step, and the seed.
pub(super) fn run_targets<I: Instance>(targets: &[Target]) -> Result {
    let seed =
        std::env::var("QUINT_SEED").unwrap_or_else(|_| format!("0x{:x}", rand::random::<u32>()));
    run_targets_with_seed::<I>(targets, &seed)
        .with_context(|| format!("reproduce this error with QUINT_SEED={seed}"))
}

fn run_targets_with_seed<I: Instance>(targets: &[Target], seed: &str) -> Result {
    let mut driver = VotorMbtDriver::<I>::new();
    for target in targets {
        let start = Instant::now();
        let witnesses = witnesses::<I>(target, seed).with_context(|| {
            format!("cannot generate the witnesses of target '{}'", target.name)
        })?;
        let quint_time = start.elapsed();
        ensure!(
            !witnesses.is_empty(),
            "no trace reached '{}' ({}) in {} samples of {} steps",
            target.name,
            target.invariant,
            target.max_samples,
            target.max_steps,
        );

        let start = Instant::now();
        let found = witnesses.len();
        for (index, trace) in witnesses.into_iter().enumerate() {
            replay_trace(&mut driver, trace).with_context(|| {
                format!(
                    "target '{}' ({}), witness {index} of {found}",
                    target.name, target.invariant
                )
            })?;
        }
        eprintln!(
            "votor MBT target '{}' ({}) of {}: {found}/{} witnesses in at most {} samples, quint \
             {}, replay {}",
            target.name,
            target.invariant,
            I::MAIN,
            target.witnesses,
            target.max_samples,
            seconds(quint_time),
            seconds(start.elapsed()),
        );
    }
    Ok(())
}

fn seconds(duration: Duration) -> String {
    format!("{:.1} s", duration.as_secs_f64())
}
