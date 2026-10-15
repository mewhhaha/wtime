"""Integrity checks for P094's provisional, disabled-native policy skeleton."""
from __future__ import annotations

from typing import Any


DEFAULT_PACKAGES = {
    "wtime-types", "wtime-decode", "wtime-validate", "wtime-runtime", "wtime-cli"
}
DESCENDANTS = {f"P{i:03d}" for i in range(95, 108)}
RESOURCE_LIMITS = {
    "maximum_function_code_bytes": 16_384,
    "maximum_module_code_bytes": 1_048_576,
    "maximum_mapping_bytes": 1_048_576,
    "maximum_temporary_compile_bytes": 2_097_152,
    "maximum_attempted_ir_instructions": 65_536,
    "maximum_source_event_expansion": 4_096,
}


def _need(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def _positive_int(value: Any, label: str) -> None:
    _need(type(value) is int and value > 0, f"{label} must be a positive integer")


def validate_policy(policy: Any) -> dict[str, Any]:
    """Require a complete no-go decision and a finite, safe future boundary."""
    _need(type(policy) is dict and policy.get("schema") == "wtime-native-policy-skeleton-v1",
          "native policy schema differs")
    _need(policy.get("task") == "P094" and policy.get("status") == "provisional",
          "P094 policy must remain provisional")
    _need(policy.get("decision") == "no_go" and policy.get("native_status") == "not_selected",
          "native portfolio must remain a no-go and not selected")
    _need(policy.get("native_enabled") is False and policy.get("aot_enabled") is False,
          "native and AOT execution must remain disabled")
    _need(policy.get("native_supported_targets") == [] and policy.get("native_measurements") == "none",
          "no-go cannot claim a supported target or native measurement")

    default = policy.get("default_policy")
    _need(type(default) is dict and default == {
        "engine": "decoded", "external_dependencies": 0,
        "unsafe": False, "native_or_aot_enabled": False,
    }, "default policy must stay decoded, safe, dependency-free, and native-disabled")

    boundary = policy.get("optional_boundary")
    _need(type(boundary) is dict
          and boundary.get("initial_target_if_reopened") == "x86_64-unknown-linux-gnu"
          and boundary.get("value_types") == ["i32", "i64"]
          and boundary.get("numeric_subset_only") is True
          and boundary.get("host_calls") is False
          and boundary.get("references") is False
          and boundary.get("native_guest_stack") is False,
          "conditional native boundary exceeds the initial numeric subset")
    fallback = boundary.get("fallback")
    _need(type(fallback) is dict and fallback == {
        "engine": "decoded", "source_fuel_exact": True,
        "cancellation_exact": True, "source_cursor_exact": True,
        "guest_state_exact": True, "partial_native_execution_before_fallback": False,
    }, "decoded fallback must preserve fuel, cancellation, cursor, and guest state")

    dependencies = policy.get("dependency_policy")
    _need(type(dependencies) is dict
          and set(dependencies.get("default_packages", [])) == DEFAULT_PACKAGES
          and dependencies.get("default_external_dependencies") == 0
          and dependencies.get("default_native_crates") == [],
          "default dependency closure must not include native or external packages")
    crates = dependencies.get("optional_crates")
    _need(type(crates) is list and len(crates) <= 2,
          "optional backend may introduce at most two internal crates")
    by_name: dict[str, dict[str, Any]] = {}
    for crate in crates:
        _need(type(crate) is dict and type(crate.get("name")) is str,
              "optional crate declaration is malformed")
        name = crate["name"]
        _need(name not in by_name and crate.get("internal") is True
              and crate.get("external_dependencies") == 0
              and type(crate.get("depends_on")) is list,
              "optional crates must be unique internal nodes without external dependencies")
        by_name[name] = crate
    _need(set(by_name) == {"wtime-exec-ir", "wtime-native"},
          "future boundary must name only wtime-exec-ir and wtime-native")
    _need(by_name["wtime-exec-ir"]["depends_on"] == []
          and by_name["wtime-native"]["depends_on"] == ["wtime-exec-ir"]
          and dependencies.get("exec_ir_depends_on_runtime") is False
          and dependencies.get("crate_graph_acyclic") is True,
          "optional crate graph must be acyclic with native depending on dependency-free exec IR")
    runtime_feature = dependencies.get("runtime_native_feature")
    _need(type(runtime_feature) is dict and runtime_feature == {
        "enabled": False, "optional_dependencies": ["wtime-native"],
        "default_membership": False,
    }, "native feature must remain opt-in and outside default members")

    unsafe = policy.get("unsafe_policy")
    _need(type(unsafe) is dict and unsafe.get("default_forbid_unsafe") is True
          and unsafe.get("scanner_exception_activated") is False
          and unsafe.get("no_default_unsafe_exemption") is True
          and unsafe.get("future_exception_scope_if_separately_approved") == "crates/wtime-native/src/**",
          "unsafe scanner exception must remain inactive and narrowly scoped")

    resources = policy.get("provisional_resource_ceilings")
    _need(type(resources) is dict
          and resources.get("status") == "design_choice_requires_evaluation"
          and resources.get("measured") is False,
          "resource ceilings must be labeled provisional and unmeasured")
    for key, expected in RESOURCE_LIMITS.items():
        _positive_int(resources.get(key), key)
        _need(resources[key] == expected, f"provisional resource ceiling changed: {key}")
    for key in ("compile_synchronous", "pre_reserve_before_emission",
                "budget_exhaustion_is_transactional", "decline_before_guest_execution",
                "checked_arithmetic"):
        _need(resources.get(key) is True, f"bounded compiler invariant missing: {key}")
    _need(resources.get("worker_pool") is False,
          "future compiler must remain synchronous without a worker pool")
    mapping = policy.get("mapping_policy")
    _need(type(mapping) is dict and mapping.get("maximum_bytes") == resources["maximum_mapping_bytes"]
          and mapping.get("writable_and_executable_simultaneously") is False
          and mapping.get("immutable_after_publication") is True
          and mapping.get("mapping_lifetime_references_audited") is True
          and mapping.get("platform_mapping_path_optional") is True
          and mapping.get("publication_is_atomic") is True,
          "executable mapping policy must be finite, never RWX, and immutable after publication")
    abi = policy.get("abi_policy")
    _need(type(abi) is dict and abi == {
        "target_abi": "System V AMD64", "stack_alignment_bytes": 16,
        "callee_saved_registers_preserved": True,
        "unwind_across_native_boundary": False,
        "entry_and_return_layout_versioned": True,
        "host_function_abi_exposed_to_guest": False,
    }, "native ABI and unwind policy must be explicit")

    thresholds = policy.get("reopen_thresholds")
    _need(type(thresholds) is dict and thresholds == {
        "source_bound_p067_p080_evidence_required": True,
        "primary_point_improvement_minimum": 0.05,
        "primary_ratio_95ci_upper_strictly_below": 1.0,
        "independent_runs_required": 2,
        "primary_gate_applies_in_each_run": True,
        "protected_ratio_95ci_upper_maximum": 1.05,
        "preparation_ratio_maximum": 1.25,
        "tiny_first_use_ratio_maximum": 1.05,
        "clean_build_seconds_maximum": 60,
        "touched_cli_leaf_seconds_maximum": 5,
        "stripped_cli_bytes_maximum": 5 * 1024 * 1024,
        "relative_clean_build_maximum": 1.25,
        "relative_leaf_build_maximum": 1.25,
        "relative_stripped_size_maximum": 1.25,
        "relative_rss_maximum": 1.25,
    }, "reopen thresholds must match the existing performance and resource gates")
    _need(policy.get("descendants") == {task: "deferred_not_selected" for task in DESCENDANTS},
          "native descendants must remain deferred and not selected")
    return policy
