#!/usr/bin/env python3
"""Refresh P127 plan metadata from current harness tests and verified protocol evidence."""
import ast
import collections
import hashlib
import json
import os
import re
import stat
import subprocess
from pathlib import Path

ROOT=Path('/workspace/wtime'); SRC=Path('/workspace/tasks'); DST=ROOT/'task-plans'
ARCH=DST/'recovery/P127-metadata-2026-10-14'; PLAN=Path('performance/TASKS.json')
TASK=Path('performance/04-specialization-and-fusion/P127-metered-unmetered-benchmark-setting.md')
MAN=ROOT/'docs/recovery/task-plan-snapshot.json'
PY_TESTS={
 'model':Path('tools/wtime-bench/tests/test_p127_fuel_setting.py'),
 'producer':Path('tools/verification/tests/test_p127_fuel.py'),
 'comparison':Path('tools/wtime-bench/tests/test_p127_comparison.py'),
 'identity':Path('tools/wtime-bench/tests/test_fast_execution_identity.py'),
}
RUST_TESTS={
 'driver':Path('tools/wtime-bench/src/fuel_policy.rs'),
 'protocol':Path('tools/wtime-bench/src/protocol.rs'),
 'scalar':Path('tools/wtime-bench/src/scalar_driver.rs'),
 'private':Path('tools/verification/producers/p127_fuel_cases.rs'),
}

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def python_test_names(path):
 tree=ast.parse(path.read_text(encoding='utf-8'))
 return sorted(n.name for n in ast.walk(tree) if isinstance(n,(ast.FunctionDef,ast.AsyncFunctionDef)) and n.name.startswith('test_'))
def rust_test_names(path):
 text=path.read_text(encoding='utf-8')
 return re.findall(r'#\[test\]\s*fn\s+(p127_[A-Za-z0-9_]+)\s*\(',text)
def main():
 old_sources=[
  (SRC/PLAN,ARCH/'pre-edit/source/performance/TASKS.json'),
  (SRC/TASK,ARCH/'pre-edit/source/performance/04-specialization-and-fusion/P127-metered-unmetered-benchmark-setting.md'),
  (DST/PLAN,ARCH/'pre-edit/snapshot/performance/TASKS.json'),
  (DST/TASK,ARCH/'pre-edit/snapshot/performance/04-specialization-and-fusion/P127-metered-unmetered-benchmark-setting.md'),
  (MAN,ARCH/'pre-edit/source/docs/recovery/task-plan-snapshot.json')]
 for live,saved in old_sources:
  if not live.is_file() or not saved.is_file() or live.read_bytes()!=saved.read_bytes():
   raise RuntimeError(f'pre-edit archive mismatch: {live}')

 model=python_test_names(ROOT/PY_TESTS['model']); producer=python_test_names(ROOT/PY_TESTS['producer'])
 comparison=python_test_names(ROOT/PY_TESTS['comparison']); identity=python_test_names(ROOT/PY_TESTS['identity'])
 rust_driver=rust_test_names(ROOT/RUST_TESTS['driver'])+rust_test_names(ROOT/RUST_TESTS['protocol'])+rust_test_names(ROOT/RUST_TESTS['scalar'])
 rust_private=rust_test_names(ROOT/RUST_TESTS['private'])
 if (len(model),len(producer),len(comparison),len(identity),len(rust_driver),len(rust_private))!=(8,8,6,7,5,4):
  raise RuntimeError(f'AST/Rust test counts differ: {len(model)}, {len(producer)}, {len(comparison)}, {len(identity)}, {len(rust_driver)}, {len(rust_private)}')

 descriptions={
 # Model tests
 'test_cli_value_requires_exact_well_typed_policy_dict':'CLI policy values accept only the exact supported typed setting.',
 'test_dependent_oracle_matches_literal_recurrence_in_admitted_wat':'Dependent workload oracle agrees with the admitted literal recurrence.',
 'test_exact_typed_policy_and_cli_contract':'Typed policy and explicit command-line policy preserve their contract.',
 'test_fixed_matrix_is_46_rows_times_four_policies_modes':'The fixed workload/stage matrix has the declared row and policy/mode cardinality.',
 'test_oracles_are_independent_and_prepare_has_null':'Independent result oracles hold, and prepare observations have no guest result.',
 'test_protocol_contract_encodes_null_vs_exact_budget':'Wire protocol distinguishes null fuel from the exact metered budget.',
 'test_registry_drift_rejects_without_mutating_source':'Registry drift is rejected without modifying shared workload input.',
 'test_validate_wire_setting_requires_presence_and_exact_json_types':'Wire validation requires a present setting with exact JSON types.',
 # Producer tests
 'test_actual_scalar_driver_wire_requires_typed_strings_and_policy_keys':'Real scalar-driver wire requires typed strings and the declared policy fields.',
 'test_full_184_policy_mode_stage_matrix_accepts_only_exact_tuple_set':'The producer accepts only the complete 184 row/stage/mode/policy tuple set.',
 'test_mock_prepare_large_fixture_is_generated_encoded_and_mvp_validated_inside_attempt':'The mock prepare_large fixture is generated, encoded, and MVP validated inside its attempt root.',
 'test_protocol_mutation_is_rejected_and_protocol_logs_are_retained':'Protocol mutation fails closed and protocol logs remain available.',
 'test_raw_command_logs_exist_before_response_mismatch_and_escape_is_blocked':'Raw command logs precede result checks and escaping outputs are rejected.',
 'test_single_case_route_and_wire_cover_call_overhead_and_prepare_large':'Single-case route and wire cover call_overhead and prepare_large.',
 'test_source_maps_use_p128_closure_and_record_protocol_mutations_abort_pre_guest':'Source mapping uses P128 closure; protocol drift aborts before guest execution.',
 'test_truthful_p128_record_shape_and_core_record_protocol_staleness_fail_closed':'P128 record shape is truthful and stale core protocol records fail closed.',
 # Comparison tests
 'test_compare_routes_reject_missing_wrong_float_and_boolean_wire_budget':'Both comparison routes reject absent, malformed, floating, and Boolean fuel settings.',
 'test_explicit_metered_and_unmetered_policies_route_through_both_compare_helpers':'Explicit metered and unmetered policies reach both comparison helpers.',
 'test_focused_observe_shares_same_setting_for_simulated_paired_sides':'Focused paired observations carry one identical selected policy on both sides.',
 'test_focused_prepare_large_is_numeric_single_case_and_returns_null':'Focused prepare_large remains a numeric single case with a null guest result.',
 'test_legacy_single_case_and_scalar_argv_remain_byte_for_byte_unchanged':'Legacy call_overhead and scalar command argv stay byte-for-byte compatible when the flag is omitted.',
 'test_unsupported_focused_family_fails_before_process_command':'Unsupported focused workload families fail before any process launch.',
 # Execution identity tests
 'test_adapter_archive_corruption_is_still_rejected':'Corrupted source archives remain rejected.',
 'test_analysis_changes_are_visible_but_do_not_stale_legacy_pairs':'Analysis-only changes are visible without invalidating legacy execution pairs.',
 'test_focused_identity_uses_task_declaration_without_registry_selection_lookup':'Focused identity follows task declaration without registry selection lookup.',
 'test_focused_producer_changes_stale_focused_pairs':'Focused producer changes invalidate focused execution pairs.',
 'test_membership_changes_and_unknown_python_modules_fail_closed':'Unclassified modules and identity membership changes fail closed.',
 'test_runtime_timer_fixture_and_oracle_edits_stale_legacy_pairs':'Runtime, timer, fixture, and oracle changes invalidate old execution pairs.',
 'test_verify_inputs_uses_live_sources_without_running_a_comparison':'Identity verification uses live source inputs without starting a comparison.',
 # Driver Rust tests
 'p127_omitted_and_explicit_unmetered_preserve_distinct_wire_identity':'Omitted and explicit unmetered CLI settings preserve distinct legacy/wire identities.',
 'p127_fixed_metered_policy_sets_fuel_without_changing_other_limits':'The fixed metered policy sets fuel and preserves other resource limits.',
 'p127_malformed_or_unsupported_fuel_arguments_are_rejected':'Malformed and unsupported fuel arguments are rejected.',
 'p127_single_case_builder_uses_selected_fuel_at_guest_source_boundary':'Single-case builder forwards the selected setting at the guest source boundary.',
 'p127_scalar_builder_uses_selected_fuel_at_guest_source_boundary':'Scalar builder forwards the selected setting at the guest source boundary.',
 # Private fuel source tests
 'p127_actual_accounting_merges_saved_fuel_once_and_preserves_error_priority':'Copied accounting helper merges saved fuel once and preserves the actual error priority.',
 'p127_scalar_policies_match_reference_and_refill_each_invocation':'Private source-test driver compares scalar results with reference and refills each invocation.',
 'p127_low_fuel_prefixes_bind_memory_cursor_and_remaining_budget':'Private source-test diagnostics bind low-fuel prefixes to memory, cursor, and remaining budget.',
 'p127_real_continuations_preserve_none_some_fuel_transitions':'Private source-test diagnostics cover None/Some fuel across real continuation transitions.',
 }
 all_names=model+producer+comparison+identity+rust_driver+rust_private
 if len(all_names)!=38 or len(set(all_names))!=38 or set(all_names)!=set(descriptions):
  raise RuntimeError('actual test names do not match reviewed P127 case descriptions')

 plan_path=SRC/PLAN; manifest=json.loads(plan_path.read_text(encoding='utf-8'))
 task=next(t for t in manifest['tasks'] if t['id']=='P127')
 if task['status']!='todo' or task['outcome']!='pending': raise RuntimeError('P127 must remain todo/pending until publisher')
 add_owned=[
  'tools/wtime-bench/src/fuel_policy.rs',
  'tools/wtime-bench/execution_identity.py',
  'tools/wtime-bench/tests/test_p127_comparison.py',
  'tools/wtime-bench/tests/test_fast_execution_identity.py',
  'tools/verification/producers/p127_driver_tests.py',
  'tools/verification/producers/p127_semantics.py',
  'tools/verification/producers/p127_fuel_cases.rs',
  'tools/verification/producers/p127_prerequisite.py',
 ]
 task['owned_paths']=[p for p in task['owned_paths'] if p!='tools/wtime-bench/src/main.rs']
 task['owned_paths'] += [p for p in add_owned if p not in task['owned_paths']]
 task['fixture_cases']=all_names
 task['minimum_focused_cases']=38
 old_intro='The independent fuel proof separately establishes that sufficient `Some(50_000_000)` and `None` return the same exact result/state for every scalar proof row, that low budgets preserve P020\'s exact first-unexecuted boundary and remaining-fuel semantics, and that the reference accounting path agrees. Cancellation, prompt retirement, operand-height behavior, fuel-error priority, and continuation resume transitions retain existing P020 semantics.'
 new_intro=old_intro+' It covers nine scalar rows × three modes (`decoded`, `compact`, `execution-reference`) × two policies × three repeats (162 guest calls), plus nine execution-reference source-event probes. Separate diagnostics cover 24 low-fuel prefixes, 12 captured cursor windows, 27 None/Some continuation tuples (54 guest calls), and five merge/three priority unit tuples using the accounting helper. The source-test-only Rust copy is diagnostic; the actual runtime tree is not edited.'
 task['input_output_contract']=task['input_output_contract'].replace(old_intro,new_intro)
 task['test_commands']=[
  'python3 -B tools/verify.py plan --contract docs/benchmarks/performance/P127-verification.json',
  'python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group policy-model --execute',
  'python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group producer-tests --execute',
  'python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group paired-policy-interface --execute',
  'python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group execution-identity --execute',
  'python3 -B tools/verify.py dev --contract docs/benchmarks/performance/P127-verification.json --group driver-protocol --execute',
  'python3 -B tools/verify.py close --contract docs/benchmarks/performance/P127-verification.json --output target/performance/P127/complete --execute',
 ]
 task['evidence_contract']='Record unchanged registry/selection identity, explicit single-command `--source-fuel none|50000000` policy and backward-compatible omitted-flag behavior, all 184 real observations by row/stage/mode/policy, fixed warm batch three, separate source-current Decoded(no runtime features)/Compact(compact-executor) production records, 38 named checks (29 Python and 9 Rust), the 162 guest calls plus nine reference source-event probes, 24 low-fuel prefixes, 12 captured cursor windows, 27 continuation tuples/54 guest calls, five merge and three priority unit tuples, 12 current P020/P018/T145 prerequisite cases from the P050 354-case receipt, and limitations. Full close contains staged and independent proof; no statistics, speed claim, or promotion.'
 task['integration_owner']='One benchmark-harness integrator owns typed fuel-policy parsing, propagation, identity, and verification proofs; `tools/wtime-bench/src/main.rs` is unchanged and not required. Runtime/core owners do not edit the actual runtime source tree.'
 task['external_constraints'] += ['Keep `tools/wtime-bench/src/main.rs` unchanged; it is not required for the policy interface.', 'Keep the copied Rust fuel-accounting source-test diagnostic separate from the actual runtime tree.']
 plan_path.write_text(json.dumps(manifest,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')

 docpath=SRC/TASK; doc=docpath.read_text(encoding='utf-8')
 # Replace owned paths with the task manifest's exact owned path list.
 owned_section=re.search(r'(## Scope and ownership\n[\s\S]*?\n)(- `tools/wtime-bench/compare\.py`[\s\S]*?)(\n\n)',doc)
 if not owned_section: raise RuntimeError('scope owned-path section not found')
 paths=''.join(f'- `{p}`\n' for p in task['owned_paths'])
 doc=doc[:owned_section.start(2)]+paths.rstrip('\n')+doc[owned_section.end(2):]
 doc=doc.replace('Runtime implementation files remain outside this task.', 'The actual runtime tree remains outside this task. `tools/wtime-bench/src/main.rs` is unchanged and not needed; the task-owned copied Rust accounting source is a test-only diagnostic, not a runtime edit.',1)
 doc=doc.replace(old_intro,new_intro,1)
 # Clarify source events, feature-specific tests, and exact CLI spelling in the recipe.
 doc=doc.replace('Preserve the setting in command argv, source-bound identity, fixed-batch identity, report metadata, and every raw observation.', 'Preserve the setting in command argv, source-bound identity, fixed-batch identity, report metadata, and every raw observation. The CLI form is `--source-fuel none|50000000`; omitted flags preserve the old wire contract.',1)
 doc=doc.replace('Keep production binaries uninstrumented and use the existing `compact-executor` feature only to expose the compact mode; `execution-reference` is confined to semantic tests/proof.', 'Keep production binaries uninstrumented: runtime-only for Decoded and `compact-executor` for Compact. Unit driver checks use runtime-only, `compact-executor`, and `execution-reference` feature configurations. The separate private fuel proof uses instrumented `compact-executor` plus `execution-reference` only for semantic diagnostics, never as a production measurement binary.',1)
 doc=doc.replace('Its full close must collect all 184 observations and the independent fuel proof without running statistics.', 'Its full close collects all 184 observations and the independent fuel proof without running statistics. The independent proof covers nine scalar rows × three modes × two policies × three repeats (162 guest calls) plus nine reference source-event probes; separate diagnostics retain 24 low-fuel prefixes, 12 cursor windows, 27 continuation tuples/54 guest calls, and five merge/three priority unit tuples.',1)
 # Replace all hypothetical test names and descriptions with AST/Rust-derived names.
 ms=doc.index('## Meaningful cases\n'); ve=doc.index('\n## Verification',ms)
 groups=[('Policy model tests',model),('Producer and independent-fuel tests',producer),('Paired policy interface tests',comparison),('Execution identity tests',identity),('Rust driver tests',rust_driver),('Source-test-only private fuel cases',rust_private)]
 block=['## Meaningful cases','']
 for title,names in groups:
  block += ['### '+title,'']
  block += [f'- `{name}`: {descriptions[name]}' for name in names]
  block.append('')
 doc=doc[:ms]+'\n'.join(block).rstrip()+doc[ve:]
 # Rewrite verification/dev commands and adjacent expectations.
 vs=doc.index('## Verification\n'); vd=doc.index('\n## Measurement and decision',vs)
 verification='''## Verification

Use development groups `policy-model`, `producer-tests`, `paired-policy-interface`, `execution-identity`, and `driver-protocol`. Full staged-runtime and independent fuel/source proof run in close. Production records are source-current, uninstrumented runtime-only Decoded and `compact-executor` Compact; `execution-reference` is confined to the semantic/source proof. The production matrix contains 184 row/stage/mode/policy observations. The independent proof contains 162 guest calls over nine scalar rows, three modes, two policies, and three repeats, plus nine reference source-event probes. It also checks 24 low-fuel prefixes, 12 captured cursor windows, 27 None/Some continuation tuples/54 guest calls, and five merge plus three priority unit tuples. These results do not add timing statistics.

```sh
'''+ '\n'.join(task['test_commands']) + '''
```

The full 38 named checks comprise 29 Python model/producer/comparison/identity cases, five Rust driver tests, and four source-test-only private fuel cases. Runtime feature checks cover runtime-only, `compact-executor`, and `execution-reference`; the separate private fuel proof uses instrumented `compact-executor` plus `execution-reference` for semantic diagnostics only. P020, P018, and T145 source-current prerequisites are inherited through the full 354-case P050 receipt. Full close runs staged and independent proofs, keeps focused paired policy identical on both future P053 sides, and performs no statistics or speed decision.'''
 doc=doc[:vs]+verification+doc[vd:]
 # Keep the plan's evidence/test command declarations exact in the verification section.
 doc=doc.replace('Record the unchanged registry and selection identities, exact fuel policy values, all 184 real observations by row/stage/mode/policy, fixed warm batch three, two source-current mode-specific production records (Decoded has no runtime features; Compact uses `compact-executor`), independent result and fuel-boundary proof, named test counts, and limitations. No task completion or timing result is claimed by this draft.',
 '''Record unchanged registry/selection identities, the exact `--source-fuel none|50000000` policy and omitted-flag compatibility, all 184 observations, warm batch three, source-current Decoded/Compact production identities, 38 named checks, independent proof dimensions (162 guest calls plus nine reference source-event probes), 24 prefixes, 12 cursor windows, 27 continuation tuples/54 guest calls, five merge and three priority unit tuples, the inherited P020/P018/T145 cases from P050, and limitations. Keep focused comparison policy the same on both future P053 sides. P127 remains todo/pending until publisher; no statistics or speed claim.''')
 docpath.write_text(doc,encoding='utf-8')
 # Sync exact modified task files; no other plan file is modified here.
 for rel in (TASK,PLAN):
  a=SRC/rel;b=DST/rel;b.parent.mkdir(parents=True,exist_ok=True);b.write_bytes(a.read_bytes());os.chmod(b,stat.S_IMODE(a.stat().st_mode))

 # Run both original validators after writing the final source and snapshot copies.
 commands=json.loads((ARCH/'argv.json').read_text())['validator_commands']; runs=[]; log=ARCH/'validator-runs.jsonl'
 for command in commands:
  script=Path(command[-1]);cwd=SRC if script.parent==SRC else script.parent
  pre={'command':command,'cwd':str(cwd),'validator_source':str(script),'validator_source_sha256':sha(script),'plan_manifest_sha256':sha(script.parent/'TASKS.json')}
  with log.open('a',encoding='utf-8') as f:f.write(json.dumps({'pre_run':pre},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())
  proc=subprocess.run(command,cwd=cwd,capture_output=True,text=True);result={**pre,'exit_code':proc.returncode,'stdout':proc.stdout,'stderr':proc.stderr};runs.append(result)
  with log.open('a',encoding='utf-8') as f:f.write(json.dumps({'result':result},sort_keys=True)+'\n');f.flush();os.fsync(f.fileno())
 snap=json.loads(MAN.read_text(encoding='utf-8'));snap.setdefault('validator_history',[]).append(snap.get('validators',[]))
 snap['validators']=[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'],'stderr':r['stderr'],'validator_source_sha256':r['validator_source_sha256'],'plan_manifest_sha256':r['plan_manifest_sha256']} for r in runs]
 counts={}
 for label,path in [('original',SRC/'TASKS.json'),('performance',SRC/PLAN)]:
  rows=json.loads(path.read_text(encoding='utf-8'))['tasks'];counts[label]={'task_total':len(rows),'status_counts':dict(sorted(collections.Counter(t.get('status','<missing>') for t in rows).items()))}
 snap['task_status_counts']=counts
 for rel in (TASK,PLAN):
  a=SRC/rel;b=DST/rel
  if a.read_bytes()!=b.read_bytes():raise RuntimeError(f'source/snapshot mismatch: {rel}')
  row=next((x for x in snap['files'] if x['path']==rel.as_posix()),None)
  if row is None:raise RuntimeError(f'manifest record missing: {rel}')
  row.update(size_bytes=a.stat().st_size,sha256=sha(a),snapshot_sha256=sha(b),byte_equal=True,mode_octal=format(stat.S_IMODE(a.stat().st_mode),'04o'))
 snap['snapshot_source_byte_equality']=all((SRC/x['path']).is_file() and (SRC/x['path']).read_bytes()==(DST/x['path']).read_bytes() and sha(SRC/x['path'])==x['sha256'] for x in snap['files'])
 MAN.write_text(json.dumps(snap,indent=2,sort_keys=True)+'\n',encoding='utf-8')
 print(json.dumps({'python_cases':[len(model),len(producer),len(comparison),len(identity)],'rust_cases':[len(rust_driver),len(rust_private)],'total_cases':len(all_names),'P127_status':task['status'],'P127_outcome':task['outcome'],'status_counts':counts,'validators':[{'command':r['command'],'exit_code':r['exit_code'],'stdout':r['stdout'].strip(),'stderr':r['stderr'].strip()} for r in runs],'snapshot_source_byte_equality':snap['snapshot_source_byte_equality']},indent=2))
 if any(r['exit_code'] for r in runs) or not snap['snapshot_source_byte_equality']:raise SystemExit(1)
if __name__=='__main__':main()
