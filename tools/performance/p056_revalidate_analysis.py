"""Reuse P056 analysis outputs after correcting a failed contract check."""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
from verification.artifacts import ArtifactStore, digest, need, normal, read_json, sha, write_new
from verification.contracts import load_spec
from verification.runner import Workflow

OUTPUTS = ("decision.json", "evidence.json", "inventory.json", "portfolio/report.json",
           "statistics.json", "variants-statistics.json")
EXPECTED_ERROR = "VerificationError: missing JSON member: /additional_statistics_computations"


def validate_prior(prior: Path, store: ArtifactStore, current_inputs: dict,
                   expected_components: dict | None = None) -> tuple[dict, dict]:
    inputs, failure = read_json(prior / "inputs.json"), read_json(prior / "failure.json")
    validate_records(inputs, failure, current_inputs, expected_components)
    commands = failure.get("commands", [])
    for row in commands:
        for stream in ("stdout", "stderr"):
            reference = row[stream]
            store.verify(reference)
            logical = (prior / (row["name"] + "." + stream)).resolve(strict=True)
            need(logical.is_relative_to(prior) and logical.is_file()
                 and sha(logical) == reference["sha256"]
                 and logical.stat().st_size == reference["bytes"],
                 "final command stream changed: " + row["name"] + "." + stream)
    # The sealing command snapshots its still-open stdout before printing its
    # final status. Validate that snapshot in the tar, and the completed stream
    # against the runner's immutable command reference above.
    need(commands[-1]["name"] == "seal-analysis", "prior analysis lacks final sealing command")
    active_streams = {"seal-analysis.stdout", "seal-analysis.stderr"}
    inventory = read_json(prior / "inventory.json")
    need(inventory.get("status") == "passed" and inventory.get("phase_root") == str(prior.resolve())
         and inventory.get("source_stable") is True
         and inventory.get("source_before") == inventory.get("source_after"),
         "prior inventory seal is invalid")
    need(inventory.get("file_count") == len(inventory["files"]), "prior inventory count differs")
    need(inventory["files"].get("inputs.json", {}).get("sha256") == sha(prior / "inputs.json"),
         "prior inventory does not bind inputs.json")
    for name, record in inventory["files"].items():
        path = (prior / name).resolve(strict=True)
        need(path.is_relative_to(prior.resolve()) and path.is_file(), "inventory path escapes prior attempt")
        need((name in active_streams or
              (path.stat().st_size == record["bytes"] and sha(path) == record["sha256"]))
             and path.stat().st_mode & 0o777 == record["mode_bits"],
             "prior inventory artifact changed: " + name)
    bundle = inventory["bundle"]
    need(sha(prior / "artifacts.tar.gz") == bundle["sha256"]
         and (prior / "artifacts.tar.gz").stat().st_size == bundle["bytes"], "prior bundle changed")
    with tarfile.open(prior / "artifacts.tar.gz", "r:gz") as archive:
        members = {m.name: m for m in archive.getmembers() if m.isfile()}
        need(set(members) == set(inventory["files"]), "prior bundle membership differs from inventory")
        for name, record in inventory["files"].items():
            stream = archive.extractfile(members[name])
            data = stream.read() if stream else b""
            need(len(data) == record["bytes"] and hashlib.sha256(data).hexdigest() == record["sha256"],
                 "prior bundle payload differs: " + name)
    return inputs, inventory


def validate_records(inputs: dict, failure: dict, current_inputs: dict,
                     expected_components: dict | None = None) -> None:
    need(failure.get("status") == "failed" and failure.get("error") == EXPECTED_ERROR,
         "prior failure differs from the approved missing-field check")
    need(digest(inputs) == failure.get("input_key"), "prior input key differs from inputs.json")
    commands = failure.get("commands", [])
    need(commands and [row.get("name") for row in commands] ==
         [row.get("name") for row in inputs["definition"]["commands"]]
         and all(row.get("exit_code") == 0 for row in commands),
         "prior analysis commands are incomplete or did not all succeed")
    expected_components = inputs["components"] if expected_components is None else expected_components
    current_components = current_inputs.get("components", {})
    need(set(current_components) == set(expected_components) | {"analysis_revalidation"},
         "current analysis identity set differs from prior plus revalidator")
    current_components = {name: value for name, value in current_components.items()
                          if name != "analysis_revalidation"}
    need(current_components == expected_components,
         "analysis components changed since the prior attempt")
    for key in ("dependencies", "environment", "lane", "runner_sources", "source_delta"):
        need(current_inputs.get(key) == inputs.get(key), "analysis input changed: " + key)
    for path, expected in inputs["tools"].items():
        need(Path(path).is_file() and sha(path) == expected, "prior executed tool changed: " + path)


def revalidate(prior: Path, output: Path, contract: Path, store_root: Path,
               repo: Path = ROOT) -> dict:
    prior, output = prior.resolve(strict=True), output.resolve(strict=True)
    store_root, repo = store_root.resolve(strict=True), repo.resolve(strict=True)
    need(prior.is_relative_to(store_root / "runs/P056/analyze"), "prior attempt is outside P056 analysis store")
    need(output.is_relative_to(store_root / "runs/P056/analyze") and output != prior,
         "output must be a fresh contained P056 analysis attempt")
    need(prior != output and not any((output / name).exists() for name in (*OUTPUTS, "revalidation.json")),
         "revalidation refuses to overwrite outputs")
    helper_copy, argv_copy = output / "p056_revalidate_analysis.executed.py", output / "revalidation.argv.json"
    shutil.copyfile(__file__, helper_copy)
    write_new(argv_copy, {"argv": sys.argv, "cwd": str(Path.cwd())})
    store = ArtifactStore(store_root)
    spec = load_spec(contract)
    workflow = Workflow(repo, store_root)
    current_inputs = workflow._inputs(spec, "analyze")
    prior_snapshot = {name: sha(prior / name) for name in ("inputs.json", "failure.json", "inventory.json", "artifacts.tar.gz")}
    inputs, inventory = validate_prior(prior, store, current_inputs)
    # Re-read identities and prior seal after validation to catch concurrent edits.
    need(workflow._inputs(spec, "analyze") == current_inputs, "current analysis inputs changed during revalidation")
    need(read_json(prior / "inventory.json") == inventory and
         prior_snapshot == {name: sha(prior / name) for name in prior_snapshot},
         "prior analysis changed during revalidation")
    for name in OUTPUTS:
        source, destination = prior / name, output / name
        need(source.is_file() and source.resolve().is_relative_to(prior), "required prior output missing: " + name)
        destination.parent.mkdir(parents=True, exist_ok=True)
        need(destination.parent.resolve().is_relative_to(output), "output parent escapes fresh attempt")
        shutil.copyfile(source, destination)
        expected = prior_snapshot["inventory.json"] if name == "inventory.json" else inventory["files"][name]["sha256"]
        need(sha(destination) == expected, "copied analysis output changed: " + name)
    need(workflow._inputs(spec, "analyze") == current_inputs
         and prior_snapshot == {name: sha(prior / name) for name in prior_snapshot},
         "analysis inputs changed while copying outputs")
    result = {"schema": "p056-analysis-revalidation-v1", "status": "passed",
              "prior_inputs": normal(prior / "inputs.json"), "prior_failure": normal(prior / "failure.json"),
              "prior_inventory": normal(prior / "inventory.json"), "prior_bundle": normal(prior / "artifacts.tar.gz"),
              "outputs": {name: normal(prior / name) for name in OUTPUTS},
              "matched_input_key": digest(inputs), "source_current": True,
              "finalized_seal_streams": {stream: read_json(prior / "failure.json")["commands"][-1][stream]
                                         for stream in ("stdout", "stderr")},
              "paired_sample_events": 0, "statistics_computations": 0,
              "bootstrap_computations": 0, "commands_rerun": 0}
    write_new(output / "revalidation.json", result)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prior", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--contract", type=Path, required=True)
    parser.add_argument("--store", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps({"status": revalidate(args.prior, args.output, args.contract, args.store)["status"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
