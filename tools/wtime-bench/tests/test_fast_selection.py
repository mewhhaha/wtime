"""Focused synthetic tests for predeclared fast selection manifests."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
TOOL = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOL))
import selection_manifest as selection  # noqa: E402


class SelectionManifestTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="wtime-fast-selection-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)
        self.registry_path = self.write("registry.json", self.registry_bytes())
        self.manifest_path = self.write("candidate.json", self.manifest_bytes())
        self.core_source_hashes = self.write_predecessor_archive()

    def write(self, relative: str, content: bytes) -> Path:
        path = (self.root / relative).resolve()
        self.assertTrue(path.is_relative_to(self.root), "synthetic output escaped fresh temporary root")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return path

    def registry(self):
        workloads = [
            {"id": "small_work", "stages": ["prepare", "first_call", "warm_call"]},
            {"id": "large_work", "stages": ["prepare", "first_call", "warm_call"]},
            {"id": "held_work", "stages": ["prepare", "first_call"]},
            {"id": "prepare_large", "stages": ["prepare"]},
            {"id": "call_overhead", "stages": ["prepare", "first_call", "load_and_call"]},
        ]
        return {"selections": {"scalar": {"workloads": [row["id"] for row in workloads]}},
                "workloads": workloads}

    def registry_bytes(self):
        return (json.dumps(self.registry(), sort_keys=True, indent=2) + "\n").encode()

    def manifest(self):
        registry = self.registry()
        full = {row["id"]: row for row in registry["workloads"]}
        declarations = [
            ("small_work", "targeted", "small", ["prepare", "first_call"]),
            ("large_work", "targeted", "large", ["prepare", "warm_call"]),
            ("held_work", "held_out", "other", ["first_call"]),
            ("prepare_large", "protected", "other", ["prepare"]),
            ("call_overhead", "protected", "other", ["first_call", "load_and_call"]),
        ]
        rows = []
        for workload_id, role, scale, stages in declarations:
            canonical = json.dumps(full[workload_id], sort_keys=True, separators=(",", ":")).encode()
            rows.append({"family": "scalar", "id": workload_id, "role": role, "scale": scale,
                         "stages": stages, "row_sha256": hashlib.sha256(canonical).hexdigest()})
        return {"schema": selection.SCHEMA, "task": "P321", "lane": "candidate",
                "rationale": "focused workload selection for synthetic verification",
                "registry_sha256": hashlib.sha256(self.registry_bytes()).hexdigest(),
                "rows": rows, "semantic_cases": ["semantic-case-a", "semantic-case-b"]}

    def manifest_bytes(self):
        return (json.dumps(self.manifest(), sort_keys=True, indent=2) + "\n").encode()

    def save_manifest(self, manifest):
        return self.write("candidate.json", (json.dumps(manifest, sort_keys=True, indent=2) + "\n").encode())

    def write_predecessor_archive(self):
        files = {}
        sources = {}
        for crate in ("wtime-runtime", "wtime-types", "wtime-decode", "wtime-validate"):
            rel = f"crates/{crate}/src/lib.rs"
            content = f"// archived {crate} source\n".encode()
            path = self.write(rel, content)
            sources[rel] = hashlib.sha256(content).hexdigest()
            archive_name = f"predecessor/source/{rel}"
            self.write(archive_name, content)
            files[archive_name.removeprefix("predecessor/")] = {
                "role": "source", "source_path": rel, "sha256": sources[rel],
            }
        for rel, content in (("Cargo.toml", b"[workspace]\n"),
                             ("Cargo.lock", b"version = 3\n"),
                             ("rust-toolchain.toml", b"[toolchain]\nchannel = 'stable'\n")):
            self.write(rel, content)
            sources[rel] = hashlib.sha256(content).hexdigest()
            archive_name = f"predecessor/source/{rel}"
            self.write(archive_name, content)
            files[archive_name.removeprefix("predecessor/")] = {
                "role": "source", "source_path": rel, "sha256": sources[rel],
            }
        manifest = {"schema": 1, "files": files}
        self.predecessor_path = self.write(
            "predecessor/manifest.json", (json.dumps(manifest, sort_keys=True, indent=2) + "\n").encode())
        return sources

    def test_valid_focused_selection_returns_original_rows_and_groups(self):
        manifest = selection.load(self.manifest_path, self.registry_path)
        selected = selection.rows(manifest, "scalar", self.registry_path)
        self.assertEqual([row["id"] for row in selected],
                         ["small_work", "large_work", "held_work", "prepare_large", "call_overhead"])
        self.assertEqual(selected[0], self.registry()["workloads"][0])
        self.assertEqual(len(selection.groups(manifest)), 8)

    def test_missing_protected_prepare_control_is_rejected(self):
        manifest = self.manifest()
        manifest["rows"] = [row for row in manifest["rows"] if row["id"] != "prepare_large"]
        with self.assertRaisesRegex(ValueError, "prepare_large"):
            selection.load(self.save_manifest(manifest), self.registry_path)

    def test_missing_held_out_workload_is_rejected(self):
        manifest = self.manifest()
        manifest["rows"] = [row for row in manifest["rows"] if row["role"] != "held_out"]
        with self.assertRaisesRegex(ValueError, "held_out"):
            selection.load(self.save_manifest(manifest), self.registry_path)

    def test_threads_family_accepts_only_declared_steady_state_group_rows(self):
        thread_ids = ["thread_small", "thread_large", "thread_held"]
        scalar_ids = ["prepare_large", "call_overhead"]
        workloads = [{"id": wid, "stages": ["steady_state_group"]} for wid in thread_ids]
        workloads += [{"id": "prepare_large", "stages": ["prepare"]},
                      {"id": "call_overhead", "stages": ["first_call", "load_and_call"]}]
        registry = {"selections": {"threads": {"workloads": thread_ids},
                                   "scalar": {"workloads": scalar_ids}}, "workloads": workloads}
        registry_path = self.write("threads-registry.json",
            (json.dumps(registry, sort_keys=True, indent=2) + "\n").encode())
        roles = [("thread_small", "targeted", "small"), ("thread_large", "targeted", "large"),
                 ("thread_held", "held_out", "other")]
        full = {row["id"]: row for row in workloads}
        rows = []
        for wid, role, scale in roles:
            raw = json.dumps(full[wid], sort_keys=True, separators=(",", ":")).encode()
            rows.append({"family": "threads", "id": wid, "role": role, "scale": scale,
                         "stages": ["steady_state_group"], "row_sha256": hashlib.sha256(raw).hexdigest()})
        for wid, stages in (("prepare_large", ["prepare"]),
                            ("call_overhead", ["first_call", "load_and_call"])):
            raw = json.dumps(full[wid], sort_keys=True, separators=(",", ":")).encode()
            rows.append({"family": "scalar", "id": wid, "role": "protected", "scale": "other",
                         "stages": stages, "row_sha256": hashlib.sha256(raw).hexdigest()})
        manifest = {"schema": selection.SCHEMA, "task": "P076", "lane": "candidate",
                    "rationale": "thread family schema fixture", "registry_sha256": hashlib.sha256(
                        registry_path.read_bytes()).hexdigest(), "rows": rows,
                    "semantic_cases": ["thread semantics"]}
        manifest_path = self.write("threads-candidate.json",
            (json.dumps(manifest, sort_keys=True, indent=2) + "\n").encode())
        self.assertEqual(len(selection.rows(selection.load(manifest_path, registry_path), "threads", registry_path)), 3)
        bad_registry = json.loads(registry_path.read_text())
        bad_registry["workloads"][0]["stages"].append("warm_call")
        bad_registry_path = self.write("threads-registry-with-extra-stage.json",
            (json.dumps(bad_registry, sort_keys=True, indent=2) + "\n").encode())
        bad = dict(manifest, registry_sha256=hashlib.sha256(bad_registry_path.read_bytes()).hexdigest())
        bad["rows"] = [dict(item) for item in rows]
        first_row = bad_registry["workloads"][0]
        canonical = json.dumps(first_row, sort_keys=True, separators=(",", ":")).encode()
        bad["rows"][0]["row_sha256"] = hashlib.sha256(canonical).hexdigest()
        with self.assertRaisesRegex(ValueError, "steady_state_group"):
            selection.load(self.write("threads-bad.json",
                (json.dumps(bad, sort_keys=True, indent=2) + "\n").encode()), bad_registry_path)

    def test_missing_targeted_large_scale_is_rejected(self):
        manifest = self.manifest()
        next(row for row in manifest["rows"] if row["id"] == "large_work")["scale"] = "other"
        with self.assertRaisesRegex(ValueError, "targeted large"):
            selection.load(self.save_manifest(manifest), self.registry_path)

    def test_missing_stage_is_rejected(self):
        manifest = self.manifest()
        next(row for row in manifest["rows"] if row["id"] == "small_work")["stages"] = []
        with self.assertRaisesRegex(ValueError, "stages must be nonempty"):
            selection.load(self.save_manifest(manifest), self.registry_path)

    def test_substituted_full_registry_row_is_rejected(self):
        manifest = self.manifest()
        next(row for row in manifest["rows"] if row["id"] == "small_work")["row_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "row_sha256 mismatch"):
            selection.load(self.save_manifest(manifest), self.registry_path)

    def test_freeze_binds_manifest_registry_predecessor_and_groups(self):
        output = (self.root / "freeze.json").resolve()
        self.assertTrue(output.is_relative_to(self.root))
        receipt = selection.freeze(self.manifest_path, self.predecessor_path, output,
                                   self.registry_path, runtime_sources=self.core_source_hashes)
        self.assertEqual(receipt["selection_count"], 5)
        self.assertEqual(receipt["runtime_sources"], self.core_source_hashes)
        self.assertEqual(selection.verify_frozen(self.manifest_path, output, self.registry_path)["task"], "P321")
        with self.assertRaisesRegex(ValueError, "already exists"):
            selection.freeze(self.manifest_path, self.predecessor_path, output, self.registry_path)

    def test_frozen_manifest_change_is_rejected(self):
        output = (self.root / "freeze.json").resolve()
        self.assertTrue(output.is_relative_to(self.root))
        selection.freeze(self.manifest_path, self.predecessor_path, output, self.registry_path)
        changed = self.manifest()
        changed["rationale"] = "changed after freeze"
        self.save_manifest(changed)
        with self.assertRaisesRegex(ValueError, "manifest_sha256 mismatch"):
            selection.verify_frozen(self.manifest_path, output, self.registry_path)

    def test_frozen_predecessor_change_is_rejected(self):
        output = (self.root / "freeze.json").resolve()
        self.assertTrue(output.is_relative_to(self.root))
        selection.freeze(self.manifest_path, self.predecessor_path, output, self.registry_path)
        self.write("predecessor/manifest.json", b'{"schema":1,"files":{}}\n')
        with self.assertRaisesRegex(ValueError, "predecessor manifest missing or changed"):
            selection.verify_frozen(self.manifest_path, output, self.registry_path)

    def test_freeze_rejects_live_core_source_changed_before_predeclaration(self):
        self.write("crates/wtime-runtime/src/lib.rs", b"// edited candidate source\n")
        output = (self.root / "freeze.json").resolve()
        self.assertTrue(output.is_relative_to(self.root))
        with self.assertRaisesRegex(ValueError, "current core source differs"):
            selection.freeze(self.manifest_path, self.predecessor_path, output, self.registry_path)
        self.assertFalse(output.exists())

    def test_verify_allows_current_core_source_to_change_after_freeze(self):
        output = (self.root / "freeze.json").resolve()
        self.assertTrue(output.is_relative_to(self.root))
        selection.freeze(self.manifest_path, self.predecessor_path, output, self.registry_path)
        self.write("crates/wtime-runtime/src/lib.rs", b"// later candidate edit\n")
        self.assertEqual(selection.verify_frozen(self.manifest_path, output, self.registry_path)["task"], "P321")

    def test_supplied_runtime_sources_must_match_archive_subset(self):
        output = (self.root / "freeze.json").resolve()
        self.assertTrue(output.is_relative_to(self.root))
        wrong = dict(self.core_source_hashes)
        wrong["crates/wtime-runtime/src/lib.rs"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "supplied runtime_sources differ"):
            selection.freeze(self.manifest_path, self.predecessor_path, output, self.registry_path,
                             runtime_sources=wrong)

    def test_freeze_rejects_new_core_source_missing_from_predecessor(self):
        self.write("crates/wtime-types/src/new_module.rs", b"// candidate-only source\n")
        output = (self.root / "freeze.json").resolve()
        self.assertTrue(output.is_relative_to(self.root))
        with self.assertRaisesRegex(ValueError, "core source membership differs"):
            selection.freeze(self.manifest_path, self.predecessor_path, output, self.registry_path)
        self.assertFalse(output.exists())

    def test_verify_rejects_tampered_frozen_runtime_source_map(self):
        output = (self.root / "freeze.json").resolve()
        self.assertTrue(output.is_relative_to(self.root))
        selection.freeze(self.manifest_path, self.predecessor_path, output, self.registry_path)
        receipt = json.loads(output.read_text())
        receipt["runtime_sources"]["crates/wtime-runtime/src/lib.rs"] = "0" * 64
        self.write("freeze.json", (json.dumps(receipt, sort_keys=True, indent=2) + "\n").encode())
        with self.assertRaisesRegex(ValueError, "runtime_sources differ"):
            selection.verify_frozen(self.manifest_path, output, self.registry_path)


if __name__ == "__main__":
    unittest.main()
