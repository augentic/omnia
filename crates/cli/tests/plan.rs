//! The `run` grammar through [`omnia_cli::plan`]: `--mount` and `--env`
//! parsing and the `--manifest` › `OMNIA_MANIFEST` › positional wasm ›
//! compiled-in source ladder.

#![cfg(not(target_arch = "wasm32"))]

use std::ffi::OsString;
use std::path::PathBuf;

use omnia_cli::{MountArg, Plan, PlanError, RunPlan, RunSource};

#[test]
fn run_mounts() {
    let plan = planned(run(
        &[
            "bin",
            "run",
            "x.wasm",
            "--mount",
            "path=workspace,name=.,writable",
            "--mount",
            "workspace",
            "--mount",
            "workspace,writable",
        ],
        None,
        false,
    ));
    assert_eq!(
        plan.mounts,
        vec![
            MountArg {
                name: ".".to_owned(),
                host_path: PathBuf::from("workspace"),
                writable: true,
            },
            // a bare path takes the name `.` and stays read-only
            MountArg {
                name: ".".to_owned(),
                host_path: PathBuf::from("workspace"),
                writable: false,
            },
            MountArg {
                name: ".".to_owned(),
                host_path: PathBuf::from("workspace"),
                writable: true,
            },
        ]
    );

    let error = usage(run(&["bin", "run", "x.wasm", "--mount", "name=.,writable"], None, false));
    assert!(error.contains("missing `path=<host-path>`"), "{error}");

    let error = usage(run(&["bin", "run", "x.wasm", "--mount", "path=x,bogus=1"], None, false));
    assert!(error.contains("unknown mount key `bogus`"), "{error}");

    let repeated = [
        ("a,path=b", "path"),
        ("path=a,path=b", "path"),
        ("path=a,name=x,name=y", "name"),
        ("path=a,writable,writable=false", "writable"),
        ("path=a,b", "path"),
        ("a,b", "path"),
    ];
    for (spec, key) in repeated {
        let error = usage(run(&["bin", "run", "x.wasm", "--mount", spec], None, false));
        assert!(error.contains(&format!("sets the {key} more than once")), "{spec}: {error}");
    }
}

#[test]
fn run_env() {
    let plan = planned(run(
        &["bin", "run", "x.wasm", "--env", "HOME", "--env", "ORDERS_*", "--", "--env", "GUEST"],
        None,
        false,
    ));
    assert_eq!(plan.env, ["HOME", "ORDERS_*"]);
    assert_eq!(plan.args, ["--env", "GUEST"], "after `--`, `--env` is the guest's");
}

#[test]
fn run_sources() {
    let manifest = RunSource::Manifest(PathBuf::from("omnia.toml"));
    let from_env = RunSource::Manifest(PathBuf::from("from_env.toml"));

    let plan = planned(run(&["bin", "run", "guest.wasm", "--manifest", "omnia.toml"], None, true));
    assert_eq!(plan.source, manifest);

    let plan = planned(run(&["bin", "run", "guest.wasm"], Some("from_env.toml"), false));
    assert_eq!(plan.source, from_env);

    let plan =
        planned(run(&["bin", "run", "--manifest", "omnia.toml"], Some("from_env.toml"), false));
    assert_eq!(plan.source, manifest);

    let plan = planned(run(&["bin", "run", "guest.wasm"], None, true));
    assert_eq!(plan.source, RunSource::Wasm(PathBuf::from("guest.wasm")));

    let plan = planned(run(&["bin", "run"], None, true));
    assert_eq!(plan.source, RunSource::CompiledIn);

    let error = fatal(run(&["bin", "run"], None, false));
    assert!(error.contains("no guest specified"), "{error}");
}

fn run(
    argv: &[&str], omnia_manifest: Option<&str>, has_compiled_in: bool,
) -> Result<Plan, PlanError> {
    let argv = argv.iter().map(OsString::from);
    omnia_cli::plan(argv, omnia_manifest.map(OsString::from), has_compiled_in)
}

// `PlanError` is not `Debug`, so each outcome is unpacked by hand
fn planned(result: Result<Plan, PlanError>) -> RunPlan {
    match result {
        Ok(Plan::Run(plan)) => plan,
        Ok(plan) => panic!("expected a run plan, got {plan:?}"),
        Err(PlanError::Usage(error)) => panic!("expected a plan, got usage: {error}"),
        Err(PlanError::Fatal(error)) => panic!("expected a plan, got fatal: {error:#}"),
    }
}

fn usage(result: Result<Plan, PlanError>) -> String {
    match result {
        Err(PlanError::Usage(error)) => error.to_string(),
        Err(PlanError::Fatal(error)) => panic!("expected a usage error, got fatal: {error:#}"),
        Ok(plan) => panic!("expected a usage error, got {plan:?}"),
    }
}

fn fatal(result: Result<Plan, PlanError>) -> String {
    match result {
        Err(PlanError::Fatal(error)) => format!("{error:#}"),
        Err(PlanError::Usage(error)) => panic!("expected a fatal error, got usage: {error}"),
        Ok(plan) => panic!("expected a fatal error, got {plan:?}"),
    }
}
