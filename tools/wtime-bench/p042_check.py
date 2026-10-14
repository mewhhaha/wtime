#!/usr/bin/env python3
"""Inert P042 prepare/check producer; --execute required. Prepare encodes/validates fixtures, never executes guests or measures."""
from __future__ import annotations
import argparse, datetime as dt, hashlib, importlib.util, json, os, re, shutil, subprocess, sys
from pathlib import Path
sys.dont_write_bytecode=True
STAGE=Path('/tmp/P042-fixture-stage')
FIXTURES=('call-ref','host-indirect','invalid-indirect','table-alias-consumer','table-alias-provider','table-mutations','table64-indirect')
SKIP_DIRS={'.git','target','node_modules','__pycache__'}
P039_TESTS={'p039_raw_float_constants_preserve_signed_zero_and_nan_payloads','p039_strict_min_max_nan_sets_and_copysign_keep_wasm_bits','p039_vector_shuffle_fuel_and_materialized_operands_match_source_events','p039_division_conversion_traps_and_fuel_priority_are_original_operations','p039_vector_block_parameters_and_results_keep_source_control_and_prefixes'}
P042_MIN_DEFAULT_TESTS=228
P041_REGRESSIONS={'p041_numeric_host_reentry_preserves_decoded_caller_gc_roots_and_growth', 'p041_tail_calls_preserve_logical_depth_and_host_suspension_barriers', 'p041_shared_module_instances_and_stores_resolve_their_own_imported_guest', 'p041_call_and_callee_cursors_materialize_the_same_canonical_state', 'p041_private_bindings_reject_foreign_sources_and_retired_or_reused_generations', 'p041_failed_start_preserves_host_side_effect_and_cannot_reuse_old_bindings', 'p041_reference_target_and_wrong_import_signature_decline_before_publication', 'p041_instance_binding_work_and_peak_bytes_decline_without_partial_publication', 'p041_argument_ranges_and_every_call_fuel_prefix_match_independent_counts', 'p041_private_binding_context_and_signature_corruption_fail_closed'}
P040_TESTS={'p040_taken_fallthrough_and_structural_cursors_keep_exact_labels_and_fuel','p040_zero_result_return_and_global_stack_peak_preserve_trap_priority','p040_unreachable_keeps_completed_local_prefix_and_charged_trap_cursor','p040_private_corruption_and_wrong_source_identity_reject_before_publication','p040_reference_valued_control_declines_whole_function_without_changing_identity','p040_private_records_match_independent_block_arities_and_targets','p040_private_work_bytes_and_indexed_arity_are_bounded_before_traversal','p040_br_table_targets_and_early_returns_have_independent_prefixes','p040_multivalue_loop_and_nested_if_match_each_source_fuel_prefix'}
P038_REGRESSIONS={'p038_integer_boundaries_locals_select_drop_execute_direct_records','p038_i64_wrapping_and_early_return_have_exact_executed_source_counts','p038_materialization_preserves_every_reachable_next_source_cursor_and_fuel','p038_reference_select_and_wide_locals_visibly_keep_decoded_fallback','p038_module_lifetime_and_pre_cancel_do_not_weaken_entry_contracts','p038_every_fuel_prefix_and_transient_stack_failure_match_decoded_and_reference'}
STAGE_LICENSES=('LICENSE.txt',)
class GateError(RuntimeError):pass

def need(ok,message):
 if not ok: raise GateError(message)

def sha(path):
 h=hashlib.sha256()
 with Path(path).open('rb') as stream:
  for chunk in iter(lambda:stream.read(1024*1024),b''): h.update(chunk)
 return h.hexdigest()

def canonical_sha(value):
 return hashlib.sha256(json.dumps(value,sort_keys=True,separators=(',',':')).encode()).hexdigest()

def write_new(path,text):
 path=Path(path); path.parent.mkdir(parents=True,exist_ok=True)
 with path.open('x',encoding='utf-8') as stream: stream.write(text)

def read_json(path):
 try: value=json.loads(Path(path).read_text(encoding='utf-8'))
 except Exception as error: raise GateError(f'invalid JSON {path}: {error}') from error
 need(isinstance(value,dict),f'expected JSON object: {path}'); return value

def runtime_map(root):
 base=root/'crates/wtime-runtime/src'
 need(base.is_dir(),'runtime source directory is missing')
 return {p.relative_to(root).as_posix():sha(p) for p in sorted(base.rglob('*.rs'))}

def archive_runtime_map(manifest):
 out={}
 for arc,row in manifest.get('files',{}).items():
  if isinstance(row,dict) and row.get('role')=='source':
   rel=row.get('source_path') or arc.removeprefix('source/')
   if rel.startswith('crates/wtime-runtime/src/'): out[rel]=row.get('sha256')
 return out

def verify_archive(archive_root,manifest):
 mapping=archive_runtime_map(manifest)
 for rel,digest in mapping.items():
  path=Path(archive_root)/'source'/rel
  need(path.is_file() and sha(path)==digest,f'archive source missing/stale: {rel}')
 return mapping

def task_row(doc,task):
 rows=[x for x in doc.get('tasks',[]) if isinstance(x,dict) and x.get('id')==task]
 need(len(rows)==1,f'expected exactly one {task} task row'); return rows[0]

def resolve_ref(root,ref,label):
 need(isinstance(ref,dict) and isinstance(ref.get('path'),str),f'{label} normal reference missing')
 path=Path(ref['path'])
 if not path.is_absolute():
  candidates=[root/path,root.parent/path]
  path=next((candidate for candidate in candidates if candidate.exists()),candidates[0])
 path=path.resolve(strict=True)
 need(path.is_file() and sha(path)==ref.get('sha256') and path.stat().st_size==ref.get('bytes'),
      f'{label} path/SHA/bytes mismatch: {path}')
 return path

def normal_ref_path(root,ref):
 """Resolve only a normal-reference path identity; do not hash its target."""
 if not isinstance(ref,dict) or not isinstance(ref.get('path'),str):return None
 path=Path(ref['path'])
 if not path.is_absolute():
  candidates=[root/path,root.parent/path]
  path=next((candidate for candidate in candidates if candidate.exists()),candidates[0])
 return path.resolve(strict=False)

def load_baseline(root):
 spec=importlib.util.spec_from_file_location('p038_fresh_baseline',root/'tools/wtime-bench/baseline.py')
 need(spec is not None and spec.loader is not None,'cannot load pinned baseline verifier')
 mod=importlib.util.module_from_spec(spec); spec.loader.exec_module(mod); return mod

def workspace_source_map(root,metadata):
 paths={root/'Cargo.toml',root/'Cargo.lock'}
 for candidate in (root/'rust-toolchain.toml',root/'rust-toolchain',root/'.cargo'):
  if candidate.is_file(): paths.add(candidate)
  elif candidate.is_dir(): paths.update(p for p in candidate.rglob('*') if p.is_file())
 for package in metadata.get('packages',[]):
  if package.get('source') is not None: continue
  manifest=Path(package['manifest_path']).resolve(); package_root=manifest.parent
  try: package_root.relative_to(root)
  except ValueError: continue
  for current,dirs,files in os.walk(package_root):
   dirs[:]=sorted(d for d in dirs if d not in SKIP_DIRS)
   for name in files:
    path=Path(current)/name
    if path.is_file(): paths.add(path)
 result={}
 for path in sorted(x for x in paths if x.is_file()):
  try: rel=path.relative_to(root).as_posix()
  except ValueError: continue
  result[rel]=sha(path)
 return result

def copy_input(src,dest):
 src=Path(src).resolve(strict=True); dest=Path(dest); dest.parent.mkdir(parents=True,exist_ok=True)
 shutil.copy2(src,dest)
 need(sha(src)==sha(dest),f'input snapshot differs: {src}')
 return {'path':str(dest.resolve()),'sha256':sha(dest),'bytes':dest.stat().st_size,'original_path':str(src),'original_sha256':sha(src)}

def run_command(run_dir,label,cmd,cwd):
 rec={'command':cmd,'cwd':str(cwd)}
 try:
  env=os.environ.copy(); env['PYTHONDONTWRITEBYTECODE']='1'
  proc=subprocess.run(cmd,cwd=cwd,env=env,text=True,capture_output=True,check=False)
  rec.update(exit_code=proc.returncode,stdout=proc.stdout,stderr=proc.stderr)
 except BaseException as exc:
  rec.update(exit_code=None,error=type(exc).__name__+': '+str(exc),stdout='',stderr='')
 write_new(run_dir/f'{label}.json',json.dumps(rec,indent=2,sort_keys=True)+'\n')
 need(rec['exit_code']==0,f'{label} failed ({rec["exit_code"]}); see {run_dir/(label+".json")}')
 return rec

def parse_named_tests(log,label,expected):
 rows=[]
 for line in log.splitlines():
  match=re.search(r'^test\s+([^ ]+)\s+\.\.\.\s+(ok|FAILED|ignored)(?:\s|$)',line.strip())
  if match: rows.append((match.group(1).rsplit('::',1)[-1],match.group(2)))
 names=[name for name,_ in rows]
 need(len(names)==len(expected) and set(names)==expected and len(set(names))==len(names),
      f'{label} expected exact named tests {sorted(expected)}, got {names}')
 need(all(status=='ok' for _,status in rows),f'{label} has a failed/ignored test: {rows}')
 summaries=[tuple(map(int,m.groups())) for m in re.finditer(r'test result: ok\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored;',log)]
 need(sum(x[0] for x in summaries)==len(expected) and all(x[1]==0 and x[2]==0 for x in summaries),
      f'{label} Cargo summaries disagree with named test output: {summaries}')
 return {'names':names,'counts':{'passed':len(names),'failed':0,'ignored':0},'cargo_summaries':summaries}

def parse_full_suite(log):
 matches=[tuple(map(int,m.groups())) for m in re.finditer(
  r'test result: ok\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored;\s+(\d+) measured;\s+(\d+) filtered out;',log)]
 need(matches and sum(x[0] for x in matches)>0 and all(x[1]==0 for x in matches),
      f'default runtime suite lacks nonzero all-passing Cargo summaries: {matches}')
 need(all(x[3]==0 and x[4]==0 for x in matches),
      f'default runtime suite was filtered or contained benchmark tests: {matches}')
 need(all(x[2]==0 for x in matches),f'default runtime suite contains ignored tests: {matches}')
 need(not re.search(r'^test\s+[^ ]+\s+\.\.\.\s+FAILED',log,re.M),
      'default runtime suite contains a failed named test')
 return {'target_summaries':matches,'passed':sum(x[0] for x in matches),'failed':sum(x[1] for x in matches),
         'ignored':sum(x[2] for x in matches),'measured':sum(x[3] for x in matches),'filtered':sum(x[4] for x in matches)}

def retain_test_binaries(root,run_dir,label,stderr):
 deps=(root/'target/debug/deps').resolve(strict=True); entries=[]
 for raw in stderr.splitlines():
  line=raw.strip()
  if not line.startswith('Running ') or ' (' not in line or not line.endswith(')'): continue
  target,sep,raw_path=line.rpartition(' (')
  if not sep or not target.startswith('Running '): continue
  executable=Path(raw_path[:-1]); executable=executable if executable.is_absolute() else root/executable
  executable=executable.resolve(strict=True)
  try: executable.relative_to(deps)
  except ValueError as error: raise GateError(f'{label} Cargo executable outside target/debug/deps: {executable}') from error
  mode=executable.stat().st_mode & 0o7777
  need(executable.is_file() and mode & 0o111,f'{label} Cargo executable missing/non-executable: {executable}')
  entries.append((target[len('Running '):],line,executable,mode))
 need(entries,f'{label} emitted no Cargo Running executable records')
 destdir=run_dir/'test-binaries'/label; destdir.mkdir(parents=True,exist_ok=False); refs=[]; seen=set()
 for target,line,exe,mode in entries:
  dest=destdir/exe.name; need(dest.name not in seen,f'{label} Cargo executable basename collision: {dest.name}'); seen.add(dest.name)
  original=sha(exe); shutil.copy2(exe,dest); copied=sha(dest); copied_mode=dest.stat().st_mode & 0o7777
  need(original==copied and mode==copied_mode,f'{label} retained executable differs: {exe}')
  refs.append({'target':target,'cargo_running_line':line,'original_path':str(exe),'original_sha256':original,
   'retained_path':str(dest.resolve()),'retained_sha256':copied,'bytes':exe.stat().st_size,'mode':oct(mode)})
 return refs

def ref(path):
 path=Path(path).resolve(strict=True)
 return {'path':str(path),'sha256':sha(path),'bytes':path.stat().st_size}

def verify_closed_predecessor(workspace,root,task_id,evidence_path,tasks_doc):
 need(task_id=='P041','P042 requires exact P041 predecessor')
 row=task_row(tasks_doc,'P041');need(row.get('status')=='done' and row.get('outcome') in ('adopted','rejected','inconclusive'),'P041 not closed')
 path=evidence_path.resolve(strict=True)
 matches=[r for r in row.get('evidence',[]) if normal_ref_path(root,r)==path]
 need(len(matches)==1,'P041 TASKS normalref path mismatch')
 need(resolve_ref(root,matches[0],'P041 exact task evidence')==path,'P041 exact TASKS normalref is stale')
 proof=read_json(path);mapping=proof.get('runtime_source_map');artifacts=proof.get('artifact_refs',{})
 need(proof.get('schema')=='p041-instance-call-evidence-v1' and proof.get('status')=='passed' and proof.get('passed') is True and proof.get('source_stable') is True and proof.get('decision')==proof.get('outcome')==row['outcome'],'P041 closure schema/outcome mismatch')
 need(isinstance(mapping,dict) and len(mapping)==28 and proof.get('current_runtime_source_map')==mapping and proof.get('runtime_source_count')==28 and proof.get('runtime_source_sha256')==canonical_sha(mapping),'P041 exact28 map/hash mismatch')
 need({'comparison','decision','check_result'}<=set(artifacts),'P041 normalrefs incomplete')
 comparison=read_json(resolve_ref(root,artifacts['comparison'],'P041 comparison'));decision=read_json(resolve_ref(root,artifacts['decision'],'P041 decision'));checked=read_json(resolve_ref(root,artifacts['check_result'],'P041 checks'))
 need(comparison.get('source_stable') is True and decision.get('task')=='P041' and decision.get('decision')==proof['decision'] and decision.get('source_identity',{}).get('comparison_report_sha256')==artifacts['comparison']['sha256'],'P041 decision/report binding mismatch')
 need(checked.get('schema')=='p041-source-bound-check-v1' and checked.get('status')=='passed' and checked.get('source_stable') is True and checked.get('runtime_source_map')==mapping,'P041 check map/status mismatch')
 return proof,mapping

def verify_stage_packet():
 entries={}
 for line in (STAGE/'SHA256SUMS').read_text().splitlines():
  digest,rel=line.split('  ',1);need(rel not in entries and re.fullmatch(r'[0-9a-f]{64}',digest),'packet manifest entry invalid')
  need((STAGE/rel).is_file() and sha(STAGE/rel)==digest,'packet hash mismatch: '+rel);entries[rel]=digest
 need(set(entries)=={n+'.wat' for n in FIXTURES}|{'manual-oracles.json','LICENSE.txt','PROVENANCE.json'},'exact sevenWAT/oracle/license/provenance packet required')
 provenance=read_json(STAGE/'PROVENANCE.json');oracle=read_json(STAGE/'manual-oracles.json')
 need(provenance.get('schema')=='p042-fixture-source-packet-v1' and provenance.get('status')=='draft_source_only_unencoded_unvalidated_unexecuted' and set(provenance.get('files',[]))==set(entries)-{'PROVENANCE.json'},'packet provenance schema/files/status mismatch')
 need({'WAT parser validation','Wasm encoding','runtime invocation','conformance or fuel measurement','cache-hit or performance measurement'}<=set(provenance.get('not_completed',[])),'packet claims unsupported evidence')
 need(oracle.get('schema')=='p042-static-manual-oracles-v1' and oracle.get('status')=='source-derived; WAT unencoded and unvalidated; no runtime observations','oracle status/schema mismatch')
 need('MIT License' in (STAGE/'LICENSE.txt').read_text() and 'Apache License' in (STAGE/'LICENSE.txt').read_text(),'dual license incomplete')
 return entries,provenance,oracle,{}

def check_runtime_delta(predecessor,current,allowed):
 need(len(predecessor)==28 and len(current)==29,'P042 must extend28 to29')
 old=set(predecessor);new=set(current);need(not old-new,'runtime sources removed')
 added=new-old;changed={n for n in old&new if predecessor[n]!=current[n]}
 need(added=={'crates/wtime-runtime/src/execution/compact/indirect.rs'},'only indirect.rs may be added')
 need(changed and changed<=allowed,'runtime delta escapes root-owned staged contract scope')
 return {'added':sorted(added),'changed':sorted(changed),'unchanged':sorted(old&new-changed)}

def snapshot(root,run_dir,args):
 refs={'producer':copy_input(Path(__file__).resolve(),run_dir/'inputs/run.executed.py')}
 for path in sorted(x for x in STAGE.rglob('*') if x.is_file()):
  rel=path.relative_to(STAGE);refs['fixture:'+rel.as_posix()]=copy_input(path,run_dir/'inputs/fixture-stage'/rel)
 for rel in ('tools/wtime-bench/baseline.py','tools/checks/build_budget.py','tools/checks/dependency_budget.py'):
  refs['helper:'+rel]=copy_input(root/rel,run_dir/'inputs/helpers'/Path(rel).name)
 for key,value in [('p041_evidence',args.p041_evidence),('p039_receipt',args.p039_receipt),('receipt',args.receipt),('test_contract',args.test_contract),('encoder_ref',args.encoder_ref),('validator_ref',args.validator_ref),('supplemental_receipt',args.supplemental_receipt)]:
  if value:refs[key]=copy_input(value,run_dir/'inputs'/key.replace('_','-'))
 write_new(run_dir/'snapshot-manifest.json',json.dumps(refs,indent=2,sort_keys=True)+'\n')
 return refs

def verify_combined_support(root, supplemental_path):
 receipt=read_json(supplemental_path)
 need(receipt.get('schema')=='p042-fixture-validation-v1' and receipt.get('status')=='passed' and receipt.get('completed') is True and receipt.get('source_stable') is True,'five supplemental fixture receipt incomplete')
 tools=receipt.get('tools',{})
 for key in ('encoder','validator'):
  tool=tools.get(key,{})
  binary=(root/tool.get('binary_path','')).resolve(strict=True)
  log_path=(root/tool.get('log_path','')).resolve(strict=True)
  need(binary.is_file() and sha(binary)==tool.get('binary_sha256') and tool.get('exit_code')==0 and sha(log_path)==tool.get('log_sha256'),'supplemental pinned tool/log mismatch')
  log=read_json(log_path);need(log.get('exit_code')==0 and log.get('command')==tool.get('command'),'supplemental tool exact command not proved')
 rows=receipt.get('fixtures');need(isinstance(rows,list) and len(rows)==5,'exact five supplemental fixtures required')
 sources={}
 for row in rows:
  relative=row.get('source_path');need(isinstance(relative,str) and relative.startswith('fixtures/scalar/p042/') and relative not in sources,'supplemental source path duplicate/outside scope')
  wat=root/'tools/wtime-bench'/relative;wasm=root/'tools/wtime-bench'/row.get('wasm_path','')
  need(wat.is_file() and wasm.is_file() and sha(wat)==row.get('source_sha256') and sha(wasm)==row.get('wasm_sha256'),'supplemental actual fixture hash mismatch')
  for key in ('encode','validate'):
   command=row.get(key,{})
   log=read_json(root/command.get('log_path',''))
   need(command.get('exit_code')==0 and log.get('exit_code')==0 and sha(root/command['log_path'])==command.get('log_sha256') and command.get('wasm_sha256')==row['wasm_sha256'],'supplemental command/log hash mismatch')
   if key=='encode':need(command.get('source_sha256')==row['source_sha256'] and 'encode' in log.get('command',[]) and str(wat) in log['command'] and str(wasm) in log['command'],'supplemental encoded source/argv mismatch')
   else:need(command.get('status')=='passed' and log.get('stdout','').strip()=='valid' and '--profile' in log.get('command',[]) and 'all-phase3-core' in log['command'] and str(wasm) in log['command'],'supplemental all-phase3 actual validation missing')
  sources[relative]=row
 registry_path=root/'tools/wtime-bench/workloads.json';registry=read_json(registry_path)
 selections=registry.get('selections',{});legacy=selections.get('scalar',{}).get('workloads',[]);combined=selections.get('scalar-p042-cache',{}).get('workloads',[])
 need(len(legacy)==85 and len(set(legacy))==85 and len(combined)==110 and len(set(combined))==110 and combined[:85]==legacy,'combined110 must preserve legacy85 ordered prefix')
 entries={row['id']:row for row in registry['workloads']};extra=combined[85:]
 need(len(extra)==25 and {entries[name]['source']['path'] for name in extra}==set(sources),'combined25 supplemental source set mismatch')
 for name in extra:
  source=entries[name]['source'];row=sources[source['path']]
  need(source.get('encoding_status')=='encoded' and source.get('sha256')==row['source_sha256'] and source.get('wasm_sha256')==row['wasm_sha256'],'installed supplemental registry encoding provenance mismatch')
 contract_path=root/'docs/benchmarks/performance/P042-contract.json';contract=read_json(contract_path)
 need(set(contract)=={'schema_version','selection','primary_workloads','protected_workloads'} and contract['selection']=='scalar-p042-cache' and len(contract['primary_workloads'])==30 and len(contract['protected_workloads'])==80 and set(contract['primary_workloads']).isdisjoint(contract['protected_workloads']) and set(contract['primary_workloads'])|set(contract['protected_workloads'])==set(combined),'combined four-key30/80 contract mismatch')
 return {'supplemental_receipt':ref(supplemental_path),'registry':ref(registry_path),'contract':ref(contract_path),'comparison_selection':'scalar-p042-cache','comparison_count':110,'primary_count':30,'protected_count':80,'diagnostic_selection':'scalar','diagnostic_count':85}

def prepare(root,workspace,run_dir,args,snapshots):
 tasks=read_json(workspace/'tasks/performance/TASKS.json')
 _,predecessor=verify_closed_predecessor(workspace,root,'P041',args.p041_evidence,tasks)
 need(runtime_map(root)==predecessor,'prepare must start from exact closed P04128')
 support=verify_combined_support(root,args.supplemental_receipt)
 pinned=read_json(args.p039_receipt);need(pinned.get('schema')=='p039-fresh-fixture-validation-v1' and pinned.get('status')=='passed','pinned P039 tools receipt invalid')
 supplemental=read_json(args.supplemental_receipt)
 for key in ('encoder','validator'):need(supplemental['tools'][key]['binary_sha256']==pinned['tools'][key]['sha256'],'supplemental tools differ from pinned P039 binaries')
 entries,packet_provenance,oracles,preserved=verify_stage_packet()
 tools={}
 for key,input_ref in [('encoder',args.encoder_ref),('validator',args.validator_ref)]:
  normal=read_json(input_ref);source=resolve_ref(root,normal,key)
  pinned_tool=pinned.get('tools',{}).get(key,pinned.get(key,{}))
  need(normal['sha256']==pinned_tool.get('sha256'),'tool hash differs from P039 pinned receipt')
  need(os.access(source,os.X_OK),'pinned '+key+' is not executable')
  saved=copy_input(source,run_dir/'inputs/tools'/key);saved['mode']=oct(source.stat().st_mode&0o7777)
  need(oct(Path(saved['path']).stat().st_mode&0o7777)==saved['mode'],'retained '+key+' mode differs')
  tools[key]=saved
 archive=root/'target/performance/P042-before';encoded=root/'target/performance/P042/fixtures'
 need(not archive.exists() and not encoded.exists(),'P042-before or P042 fixture output already exists; refusing overwrite')
 archive_rec=run_command(run_dir,'fresh-compact-archive',
  ['python3','-B','tools/wtime-bench/baseline.py','--mode','compact','--output',str(archive)],root)
 manifest=read_json(archive/'manifest.json')
 need(manifest.get('mode')=='compact' and verify_archive(archive,manifest)==predecessor,
      'fresh P042-before does not bind the exact P039 runtime map')
 need(load_baseline(root).verify_archive(archive)==manifest,'independent P042-before archive verification failed')
 encoded.mkdir(parents=True,exist_ok=False);fixtures={}
 for name in FIXTURES:
  wat=STAGE/(name+'.wat');wasm=encoded/(name+'.wasm');commands={}
  for kind,argv in [('encode',[tools['encoder']['path'],'encode',str(wat),'-o',str(wasm)]),
                    ('validate',[tools['validator']['path'],'--profile','all-phase3-core','check',str(wasm)])]:
   rec=run_command(run_dir,kind+'-'+name,argv,root)
   if kind=='validate':need(rec['stdout'].strip()=='valid','validator did not emit exact valid result for '+name)
   commands[kind]={'command':argv,'exit_code':rec['exit_code'],'log':ref(run_dir/(kind+'-'+name+'.json'))}
  need(wasm.is_file() and wasm.stat().st_size>8,'encoded P042 Wasm missing or empty: '+name)
  fixtures[name]={'source':ref(wat),'wasm':ref(wasm),'commands':commands,
   'manual_oracle':ref(STAGE/'manual-oracles.json'),'provenance':ref(STAGE/'PROVENANCE.json')}
 need(runtime_map(root)==predecessor,'runtime source changed while preparing P042 archive and fixtures')
 for key,row in tools.items():need(sha(row['path'])==row['sha256'],'retained '+key+' changed during prepare')
 receipt={'schema':'p042-fresh-fixture-validation-v1','status':'passed','phase':'prepare',
  'runtime_predecessor':predecessor,'combined_support':support,'p041_evidence':ref(args.p041_evidence),'p039_receipt':ref(args.p039_receipt),
  'before_manifest':ref(archive/'manifest.json'),'archive_command':ref(run_dir/'fresh-compact-archive.json'),
  'tools':tools,'fixtures':fixtures,'manual_oracles':ref(STAGE/'manual-oracles.json'),
  'provenance':ref(STAGE/'PROVENANCE.json'),'licenses':{'LICENSE.txt':ref(STAGE/'LICENSE.txt')},
  'packet_manifest':ref(STAGE/'SHA256SUMS'),'packet_history':snapshots,
  'packet_source_status':packet_provenance.get('status'),'oracle_status':oracles.get('status'),
  'snapshots':snapshots,'run_dir':str(run_dir)}
 write_new(run_dir/'receipt.json',json.dumps(receipt,indent=2,sort_keys=True)+'\n');return receipt

def check_fixture_log(root,name,kind,row):
 log=read_json(resolve_ref(root,row.get('log'),name+' '+kind+' log'))
 need(log.get('exit_code')==0 and log.get('command')==row.get('command'),name+' '+kind+' command log differs')
 if kind=='validate':need(log.get('stdout','').strip()=='valid',name+' validator output is not exactly valid')

def check(root,workspace,run_dir,args,snapshots):
 receipt=read_json(args.receipt)
 need(receipt.get('schema')=='p042-fresh-fixture-validation-v1' and receipt.get('status')=='passed' and receipt.get('phase')=='prepare',
      'P042 prepare receipt is not complete')
 tasks=read_json(workspace/'tasks/performance/TASKS.json')
 p041_path=resolve_ref(root,receipt.get('p041_evidence'),'P042 P041 evidence')
 _,predecessor=verify_closed_predecessor(workspace,root,'P041',p041_path,tasks)
 need(receipt.get('runtime_predecessor')==predecessor,'receipt predecessor differs')
 resolve_ref(root,receipt.get('p039_receipt'),'pinned P039 receipt')
 supplemental_path=resolve_ref(root,receipt.get('combined_support',{}).get('supplemental_receipt'),'supplemental5 receipt')
 need(verify_combined_support(root,supplemental_path)==receipt.get('combined_support'),'support/registry/contract changed after baseline preparation')
 need(set(receipt.get('tools',{}))=={'encoder','validator'},'P042 receipt must bind encoder and validator')
 for key,row in receipt['tools'].items():
  tool=resolve_ref(root,row,'retained '+key)
  need(oct(tool.stat().st_mode&0o7777)==row.get('mode') and os.access(tool,os.X_OK),'retained P042 '+key+' identity/mode invalid')
 entries,packet_provenance,oracles,preserved=verify_stage_packet()
 need(receipt.get('licenses')=={'LICENSE.txt':ref(STAGE/'LICENSE.txt')},'P042 source-packet license ref differs')
 need(receipt.get('packet_source_status')==packet_provenance.get('status') and receipt.get('oracle_status')==oracles.get('status'),
      'P042 source-only packet status differs from receipt')
 for label,rel in [('manual_oracles','manual-oracles.json'),('provenance','PROVENANCE.json'),('packet_manifest','SHA256SUMS')]:
  need(resolve_ref(root,receipt.get(label),'P042 '+label)==STAGE/rel,'P042 '+label+' path differs from staged packet')
 need(set(receipt.get('fixtures',{}))==set(FIXTURES),'P042 receipt must bind exactly seven fixtures')
 for name,row in receipt['fixtures'].items():
  for key in ('source','wasm','manual_oracle','provenance'):resolve_ref(root,row.get(key),name+' '+key)
  need(row['source']['sha256']==entries[name+'.wat'],'P042 source differs from packet SHA256SUMS: '+name)
  need(row['manual_oracle']==receipt['manual_oracles'] and row['provenance']==receipt['provenance'],
       'P042 fixture row packet references differ from receipt')
  need(set(row.get('commands',{}))=={'encode','validate'},'P042 fixture requires actual encode and validate command rows')
  expected_commands={
   'encode':[receipt['tools']['encoder']['path'],'encode',row['source']['path'],'-o',row['wasm']['path']],
   'validate':[receipt['tools']['validator']['path'],'--profile','all-phase3-core','check',row['wasm']['path']]}
  for kind,command in row['commands'].items():
   need(command.get('command')==expected_commands[kind],name+' '+kind+' command does not use pinned tools/source/Wasm')
   check_fixture_log(root,name,kind,command)
 archive=Path(resolve_ref(root,receipt.get('before_manifest'),'P042-before manifest')).parent
 manifest=read_json(archive/'manifest.json')
 need(verify_archive(archive,manifest)==predecessor and load_baseline(root).verify_archive(archive)==manifest,
      'P042-before archive does not verify against P039 predecessor')
 contract_path=args.test_contract;contract=read_json(contract_path);names=contract.get('named_tests')
 need(isinstance(names,list) and len(names)>=3 and len(names)==len(set(names)),'root dynamic test names min3/unique required')
 expected_names={name.rsplit('::',1)[-1] for name in names}
 need(all(name.startswith('p042_') for name in expected_names),'wrong P042 test prefix')
 public={n.rsplit('::',1)[-1] for n in contract.get('public_tests',[])};private={n.rsplit('::',1)[-1] for n in contract.get('private_tests',[])}
 need(public and private and public|private==expected_names and not public&private,'root must partition public/private exact tests')
 test_sources=contract.get('test_sources')
 need(isinstance(test_sources,list) and test_sources,'P042 test contract has no test source references')
 source_paths=[resolve_ref(root,row,'P042 test source') for row in test_sources]
 need(any(p.name=='p042.rs' for p in source_paths) and any(p.name=='p042_private.rs' for p in source_paths),
      'P042 contract must bind both public and private performance test sources')
 need(contract.get('nonzero_compact_coverage_required') is True,'P042 test contract must require compact call coverage')
 current=runtime_map(root)
 task=task_row(tasks,'P042');allowed={p for p in task.get('owned_paths',[]) if p.startswith('crates/wtime-runtime/src/')}
 need(allowed,'P042 staged contract ownership absent')
 delta=check_runtime_delta(predecessor,current,allowed)
 durable=root/'crates/wtime-runtime/tests/fixtures/performance/p042';provenance_path=durable/'provenance.json'
 durable_provenance=read_json(provenance_path)
 need(durable_provenance.get('schema')=='p042-durable-fixtures-v1' and durable_provenance.get('status')=='freshly_encoded_and_validated',
      'P042 durable fixture provenance is missing or not validated')
 need(durable_provenance.get('receipt')==ref(args.receipt) and durable_provenance.get('implementation_sources')==current,
      'P042 durable fixture receipt/source identity mismatch')
 need(durable_provenance.get('fixtures')==receipt['fixtures'] and durable_provenance.get('tools')==receipt['tools'] and
      durable_provenance.get('manual_oracles')==receipt['manual_oracles'] and durable_provenance.get('licenses')==receipt['licenses'],
      'P042 durable fixture provenance differs from prepare receipt')
 for name,row in receipt['fixtures'].items():
  for key,suffix in [('source','wat'),('wasm','wasm')]:
   path=durable/(name+'.'+suffix)
   need(path.is_file() and sha(path)==row[key]['sha256'] and path.stat().st_size==row[key]['bytes'],
        'durable P042 fixture bytes differ: '+name+'.'+suffix)
 meta=run_command(run_dir,'workspace-metadata-before',['cargo','metadata','--locked','--format-version=1'],root)
 before=workspace_source_map(root,json.loads(meta['stdout']));tests={}
 write_new(run_dir/'workspace-source-before.json',json.dumps(before,indent=2,sort_keys=True)+'\n')
 write_new(run_dir/'source-before.json',json.dumps(current,indent=2,sort_keys=True)+'\n')
 commands=[('p042-focused-tests',['cargo','test','--locked','-p','wtime-runtime','--features','compact-executor,execution-reference','p042_','--','--nocapture'],expected_names),
  ('p041-regression-tests',['cargo','test','--locked','-p','wtime-runtime','--features','compact-executor,execution-reference','p041_','--','--nocapture'],P041_REGRESSIONS),
  ('p040-regression-tests',['cargo','test','--locked','-p','wtime-runtime','--features','compact-executor,execution-reference','p040_','--','--nocapture'],P040_TESTS),
  ('p039-regression-tests',['cargo','test','--locked','-p','wtime-runtime','--features','compact-executor,execution-reference','p039_','--','--nocapture'],P039_TESTS),
  ('p038-regression-tests',['cargo','test','--locked','-p','wtime-runtime','--features','compact-executor,execution-reference','p038_','--','--nocapture'],P038_REGRESSIONS),
  ('default-runtime-suite',['cargo','test','--locked','-p','wtime-runtime'],None)]
 for label,command,expected in commands:
  rec=run_command(run_dir,label,command,root);log=rec['stdout']+'\n'+rec['stderr']
  result=parse_named_tests(log,label,expected) if expected is not None else parse_full_suite(log)
  if label=='default-runtime-suite':
   need(result['passed']>=P042_MIN_DEFAULT_TESTS,f'default runtime suite passed fewer than {P042_MIN_DEFAULT_TESTS}')
   need(result['ignored']==0,'default runtime suite contains ignored tests')
  tests[label]={'results':result,'retained_test_binaries':retain_test_binaries(root,run_dir,label,rec['stderr']),'command_log':ref(run_dir/(label+'.json'))}
 builds={}
 for key,flags in [('default',[]),('optional',['--features','wtime-runtime/compact-executor'])]:
  path=root/('target/performance/P042/'+('build.json' if key=='default' else 'optional-build.json'))
  need(not path.exists(),'P042 '+key+' build report already exists; refusing overwrite')
  run_command(run_dir,key+'-build-budget',['python3','-B','tools/checks/build_budget.py',*flags,'--output',str(path)],root)
  report=read_json(path)
  need(report.get('source_inputs')==report.get('source_inputs_after')==before and report.get('source_stable') is True and
       report.get('budget_pass') is True and report.get('mode')==('default' if key=='default' else 'optional-feature') and
       report.get('features')==([] if key=='default' else ['wtime-runtime/compact-executor']),
       'P042 '+key+' build budget/source binding failed')
  builds[key]=ref(path)
 dep=run_command(run_dir,'dependency-budget',['python3','-B','tools/checks/dependency_budget.py'],root)
 need('Production dependency/unsafe budget: 0 violations' in dep['stdout']+dep['stderr'],
      'P042 zero dependency/unsafe violations were not proved')
 meta=run_command(run_dir,'workspace-metadata-after',['cargo','metadata','--locked','--format-version=1'],root)
 after=workspace_source_map(root,json.loads(meta['stdout']));runtime_after=runtime_map(root)
 write_new(run_dir/'workspace-source-after.json',json.dumps(after,indent=2,sort_keys=True)+'\n')
 write_new(run_dir/'source-after.json',json.dumps(runtime_after,indent=2,sort_keys=True)+'\n')
 need(after==before and runtime_after==current,'workspace/runtime source changed during P042 check')
 result={'schema':'p042-source-bound-check-v1','phase':'check','status':'passed','passed':True,'source_stable':True,
  'runtime_source_map':current,'runtime_source_count':len(current),'runtime_source_delta':delta,
  'combined_support':receipt['combined_support'],'p041_evidence':ref(p041_path),'p039_receipt':receipt['p039_receipt'],'receipt':ref(args.receipt),'test_contract':ref(contract_path),
  'tests':tests,'builds':builds,'dependency_log':ref(run_dir/'dependency-budget.json'),'workspace_source_map':before,
  'source_snapshots':{name:ref(run_dir/name) for name in ['workspace-source-before.json','workspace-source-after.json','source-before.json','source-after.json']},
  'fixture_provenance':ref(provenance_path),'snapshots':snapshots,'run_dir':str(run_dir),
  'comparison_output_policy':'target/performance/P042/comparison; this helper produces no timing measurements'}
 write_new(run_dir/'result.json',json.dumps(result,indent=2,sort_keys=True)+'\n');return result

def main():
 ap=argparse.ArgumentParser(description=__doc__)
 ap.add_argument('--workspace-root',type=Path,required=True);ap.add_argument('--phase',choices=['prepare','check'],required=True);ap.add_argument('--execute',action='store_true')
 for flag in ('p041-evidence','p039-receipt','encoder-ref','validator-ref','receipt','test-contract','supplemental-receipt'):ap.add_argument('--'+flag,type=Path)
 args=ap.parse_args()
 if not args.execute:print('inert: --execute required; no archive, encoding, validation, tests, or build action');return 0
 workspace=args.workspace_root.resolve(strict=True);root=workspace/'wtime';need((root/'Cargo.toml').is_file(),'wtime repository missing')
 if args.phase=='prepare':need(all(x is not None for x in (args.p041_evidence,args.p039_receipt,args.encoder_ref,args.validator_ref,args.supplemental_receipt)),
                               'prepare requires P041 closure/P039 receipt and pinned encoder/validator refs')
 else:need(args.receipt is not None and args.test_contract is not None,'check requires receipt and P042 test contract')
 base=root/'target/performance/P042'/args.phase;base.mkdir(parents=True,exist_ok=True);index=1
 while (base/f'attempt-{index:04}').exists():index+=1
 run_dir=base/f'attempt-{index:04}';run_dir.mkdir(exist_ok=False)
 try:
  snapshots=snapshot(root,run_dir,args)
  result=prepare(root,workspace,run_dir,args,snapshots) if args.phase=='prepare' else check(root,workspace,run_dir,args,snapshots)
  print(json.dumps(result,sort_keys=True));return 0
 except BaseException as exc:
  write_new(run_dir/'failure.json',json.dumps({'status':'failed_closed','error':type(exc).__name__+': '+str(exc)},indent=2)+'\n');print(str(exc),file=sys.stderr);return 1
if __name__=='__main__':raise SystemExit(main())
