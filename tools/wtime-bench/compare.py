#!/usr/bin/env python3
"""Pair independently built wtime processes over one immutable fixture set."""
import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
STAGES = ['prepare', 'instantiate', 'first_call', 'warm_call', 'load_and_call']
MEMORY_STAGED_FAMILIES = {'memory_widths_predictable', 'memory_widths_shuffled',
                          'memory_widths_read_scan', 'memory_copy_distinct_seeded',
                          'memory_fill_seeded'}
LEGACY_SCALAR_CONTROL_IDS = {'call_overhead', 'integer_loop', 'branch_calls'}
INPUTS = {'call_overhead':100, 'integer_loop':10000, 'memory_fill_scan':4096,
          'branch_calls':5000, 'prepare_large':0}

class ComparisonError(ValueError): pass

def sha(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def _read_build_growth_explanation(path):
    if path is None:
        return None
    source=Path(path).resolve()
    try:
        raw=source.read_bytes()
    except OSError as exc:
        raise ComparisonError(f'cannot read build-growth explanation: {exc}') from exc
    if not raw or len(raw)>16*1024:
        raise ComparisonError('build-growth explanation must be nonempty and at most 16 KiB')
    try:
        content=raw.decode('utf-8')
    except UnicodeDecodeError as exc:
        raise ComparisonError('build-growth explanation must be valid UTF-8') from exc
    if not content.strip():
        raise ComparisonError('build-growth explanation must contain non-whitespace text')
    try:
        display_path=source.relative_to(ROOT).as_posix()
    except ValueError:
        display_path=str(source)
    return {'content':content,'path':source,'metadata':{
        'relative_build_growth_explanation':content,
        'relative_build_growth_explanation_source':{
            'path':display_path,'sha256':hashlib.sha256(raw).hexdigest()}}}

def _common_calibrated_batch(fixture, observations):
    batches=[row.get('batch') for row in observations]
    if (not batches or any(type(batch) is not int or batch<1 or batch>100000 for batch in batches)):
        raise ComparisonError('warm calibration returned an invalid batch')
    # Memory warm calls preserve the minimum calibrated duration on both sides;
    # scalar compatibility retains the established smaller common batch.
    return max(batches) if fixture.get('memory') else min(batches)

def _verify_build_growth_explanation(record):
    if record is None:
        return
    expected=record['metadata']['relative_build_growth_explanation_source']['sha256']
    try:
        actual=sha(record['path'])
    except OSError as exc:
        raise ComparisonError(f'cannot re-read build-growth explanation: {exc}') from exc
    if actual!=expected:
        raise ComparisonError('build-growth explanation changed during measurement')

def sources(*, include_memory=False, include_simd=False, include_representative=False, include_integration=False):
    """Hash candidate runtime, harness, workload schema and imported helpers."""
    paths = [ROOT/'Cargo.toml', ROOT/'Cargo.lock', ROOT/'rust-toolchain.toml',
             HERE/'Cargo.toml', HERE/'Cargo.lock', HERE/'workloads.json']
    paths += [ROOT/'.cargo/config.toml', ROOT/'.cargo/config',
              HERE/'.cargo/config.toml', HERE/'.cargo/config']
    paths += list((ROOT/'crates').rglob('*.rs')) + list((ROOT/'crates').rglob('Cargo.toml'))
    paths += list((HERE/'src').rglob('*.rs'))
    # Workload selection and validation are executable inputs. Include all
    # benchmark Python modules so imported adapters/helpers cannot drift unseen.
    paths += [p for p in HERE.rglob('*.py')
              if 'target' not in p.relative_to(HERE).parts
              and 'tests' not in p.relative_to(HERE).parts]
    paths += list((ROOT/'tools'/'wtime-conformance').rglob('*.rs'))
    paths += list((ROOT/'tools'/'wtime-conformance').rglob('Cargo.toml'))
    paths += list((HERE/'fixtures'/'scalar').rglob('*.wat'))
    # GC staged measurements bind WAT sources and checked warm Wasm fixtures.
    paths += list((HERE/'fixtures'/'gc').rglob('*.wat'))
    paths += list((HERE/'fixtures'/'gc').rglob('*.wasm'))
    paths += list((HERE/'fixtures'/'exceptions').rglob('*.wat'))
    paths += [ROOT/'docs/benchmarks/performance'/name for name in
              ('P033-contract.json', 'P034-contract.json',
               'P034-measurement-contract.json', 'P034-registry-preservation.json')]
    runtime_gc = ROOT/'crates/wtime-runtime/tests/fixtures/performance/gc'
    paths += list(runtime_gc.glob('*.wat')) + list(runtime_gc.glob('*.wasm'))
    if include_representative:
        paths += list((HERE/'fixtures'/'representative').glob('*.wat'))
        paths += list((HERE/'fixtures'/'representative').glob('*.wasm'))
    if include_memory:
        paths += list((HERE/'fixtures'/'memory').rglob('*.wat'))
    if include_simd:
        paths += list((HERE/'fixtures'/'simd').rglob('*.wat'))
    if include_integration:
        paths += list((HERE/'fixtures'/'integration_p113').glob('*.wat'))
        paths += list((HERE/'fixtures'/'integration_p113').glob('*.wasm'))
        paths += list((HERE/'fixtures'/'integration_p113').glob('*.json'))
    return {str(p.relative_to(ROOT)):sha(p) for p in sorted(set(paths)) if p.is_file()}


def selection_uses_oracle_module(selection, module):
    """Return whether any selected row delegates its oracle to a module."""
    if selection is None:
        return False
    import workloads
    try:
        entries = workloads.selection(selection)
    except Exception as exc:
        raise ComparisonError(f'cannot resolve workload selection {selection!r}: {exc}') from exc
    return any(entry.get('oracle', {}).get('module') == module for entry in entries)

def _validate_simd_model_rows(selected_entries, modeled_entries):
    """Require exact selected SIMD rows from the independent model."""
    by_id={entry.get('id'):entry for entry in modeled_entries}
    selected_by_id={entry.get('id'):entry for entry in selected_entries}
    if (len(by_id)!=len(modeled_entries) or len(selected_by_id)!=len(selected_entries)
            or set(by_id)!=set(selected_by_id)):
        raise ComparisonError('SIMD selection does not cover the complete simd_workloads model')
    for case,entry in by_id.items():
        if selected_by_id[case]!=entry:
            raise ComparisonError(f'{case}: full SIMD registry row differs from independent model')


def _simd_observation(exe, fixture, entry, input_value, raw_value, allowed, stage, batch, mode,
                      profile, cpu, *, model, expected_batch=None):
    """Run one typed SIMD observation and verify its independent result contract."""
    import simd_scaling
    allowed=sorted(set(allowed),key=lambda item:int(item.removeprefix('i32:')))
    if not allowed or any(not item.startswith('i32:') for item in allowed):
        raise ComparisonError(f'{entry["id"]}: SIMD oracle must produce i32 results')
    command='--simd-memory-case' if isinstance(entry.get('memory_expectation'),dict) else (
        '--typed-allowed-case' if len(allowed)>1 else '--typed-case')
    expected_wire='|'.join(allowed)
    args=[command,fixture,entry['id'],input_value,expected_wire,stage,str(batch),mode,profile]
    row=process(exe,args,cpu)
    if (row.get('schema_version')!=2 or row.get('case')!=entry['id']
            or row.get('stage')!=stage or row.get('input')!=input_value
            or row.get('expected')!=expected_wire or row.get('mode')!=mode
            or row.get('profile')!=profile):
        raise ComparisonError(f'{entry["id"]}/{stage}: typed SIMD response identity/settings mismatch')
    wire_result=row.get('result')
    if wire_result not in allowed:
        raise ComparisonError(f'{entry["id"]}/{stage}: result is outside independent oracle set')
    try:
        observed=int(wire_result.removeprefix('i32:'))
    except (AttributeError,ValueError) as exc:
        raise ComparisonError(f'{entry["id"]}/{stage}: malformed typed SIMD result') from exc
    ns=row.get('ns_per_operation'); actual_batch=row.get('batch')
    if isinstance(ns,bool) or not isinstance(ns,(int,float)) or not math.isfinite(ns) or ns<=0:
        raise ComparisonError(f'{entry["id"]}/{stage}: timing must be finite and positive')
    if isinstance(actual_batch,bool) or not isinstance(actual_batch,int) or not 1<=actual_batch<=100000:
        raise ComparisonError(f'{entry["id"]}/{stage}: batch must be in 1..100000')
    if stage!='warm_call' and actual_batch!=1:
        raise ComparisonError(f'{entry["id"]}/{stage}: cold stage must run exactly one call')
    if expected_batch is not None and actual_batch!=expected_batch:
        raise ComparisonError(f'{entry["id"]}/{stage}: paired batch mismatch')
    allowed_values=[int(value.removeprefix('i32:')) for value in allowed]
    normalized=allowed_values[0] if len(allowed_values)==1 else {'allowed_results':allowed_values}
    result={'ns_per_operation':float(ns),'batch':actual_batch,'result':normalized,
            'wire_result':observed,'mode':mode,
            'settings':{'feature_profile':profile,'features':entry['required_features'],
                        'fuel':False,'cancellation_quantum':1}}
    if isinstance(entry.get('memory_expectation'),dict):
        expected_memory=model.expected_memory(entry['id'],raw_value)
        try:
            result.update(simd_scaling._memory_proof(entry,row,stage))
        except simd_scaling.MeasurementError as exc:
            raise ComparisonError(str(exc)) from exc
        result['memory_oracle']=simd_scaling._memory_oracle_record(entry)
        if expected_memory.get('result') not in allowed_values:
            raise ComparisonError(f'{entry["id"]}: memory oracle disagrees with result oracle')
        result['memory_oracle']['result']=expected_memory['result']
    return result


def _calibrate_simd(exe, fixture, entry, input_value, raw_value, allowed, mode,
                    profile, cpu, *, model, target_ns=20_000_000, max_batch=100000,
                    max_attempts=8):
    """Grow typed warm-call batches until measured duration reaches target_ns."""
    requested=0
    for attempt in range(max_attempts):
        row=_simd_observation(exe,fixture,entry,input_value,raw_value,allowed,
                              'warm_call',requested,mode,profile,cpu,model=model)
        actual=row['batch']
        if requested and actual!=requested:
            raise ComparisonError(f'{entry["id"]}: warm calibration response batch differs from request')
        duration=actual*row['ns_per_operation']
        if duration>=target_ns:
            return row
        if actual>=max_batch:
            break
        grown=math.ceil((target_ns/row['ns_per_operation'])*1.1)
        requested=min(max_batch,max(actual+1,grown))
    raise ComparisonError(f'{entry["id"]}: warm calibration did not reach {target_ns}ns '
                          f'within {max_batch} calls and {max_attempts} attempts')


def _validate_simd_capability(side, capability, mode, profiles):
    required_types={'i32','i64','f32_bits','f64_bits','v128_bits'}
    if (capability.get('schema_version')!=2 or capability.get('engine')!='wtime'
            or capability.get('instrumented') is not False
            or mode not in capability.get('modes',[])
            or not set(profiles).issubset(set(capability.get('features',[])))
            or not required_types.issubset(set(capability.get('types',[])))):
        raise ComparisonError(f'{side}: protocol does not prove uninstrumented typed SIMD profile/type support')


def _run_simd(args, growth_explanation=None):
    """Paired P026 path for the complete independent SIMD workload model."""
    import checkpoint
    import baseline
    import workloads
    import simd_workloads
    import simd_scaling

    checkpoint.Journal.preflight(args.output)
    manifest=baseline.verify_archive(args.baseline)
    protocol=HERE/'src'/'protocol.rs'
    baseline_info=_baseline_identity(args.baseline,manifest,protocol,baseline)
    if baseline_info['kind']=='legacy-api-adapter':
        raise ComparisonError('P026 SIMD selection requires a current-source archive with the typed driver')
    frozen=baseline_info['frozen']; baseline_exe=baseline_info['executable']
    baseline_mode=baseline_info['mode']
    _verify_scalar_harness_sources(frozen,manifest)
    selected=workloads.selection(args.selection)
    simd_entries=[entry for entry in selected
                  if entry.get('oracle',{}).get('module')=='simd_workloads.py']
    if len(simd_entries)!=len(selected):
        raise ComparisonError('P026 selection must contain only registered SIMD model rows')
    _validate_simd_model_rows(simd_entries,simd_workloads.entries())

    candidate=args.candidate.resolve(); encoder=ROOT/'target'/'debug'/'wtime-conformance'
    if not candidate.is_file() or not os.access(candidate,os.X_OK):
        raise ComparisonError('candidate benchmark executable is missing or not executable')
    if not encoder.is_file() or not os.access(encoder,os.X_OK):
        raise ComparisonError('existing wtime-conformance encoder is missing or not executable')
    allowed_cpus=sorted(os.sched_getaffinity(0)); cpu=args.cpu if args.cpu is not None else allowed_cpus[0]
    if cpu not in allowed_cpus: raise ComparisonError('CPU outside available affinity')
    encoder_hash=sha(encoder)

    args.output.mkdir(parents=True,exist_ok=False)
    encoded_dir=args.output/'fixtures'/'simd'
    try:
        encoded=simd_workloads.encode_fixtures(encoder,encoded_dir)
    except Exception as exc:
        raise ComparisonError(f'SIMD fixture encoding failed: {exc}') from exc
    fixtures=[]
    for entry in simd_entries:
        source=entry.get('source',{}); wat=HERE/source.get('path','')
        wasm=encoded_dir/(Path(source.get('path','')).stem+'.wasm')
        if (source.get('encoding_status')!='encoded' or not wat.is_file() or not wasm.is_file()
                or sha(wat)!=source.get('sha256') or sha(wasm)!=source.get('wasm_sha256')
                or encoded.get(wat.name)!=source.get('wasm_sha256')):
            raise ComparisonError(f'{entry.get("id")}: SIMD WAT/Wasm encoder provenance mismatch')
        raw_value=next((entry.get('oracle_arguments',{}).get(key)
                        for key in ('n','case','index','vectors','value')
                        if key in entry.get('oracle_arguments',{})),None)
        if raw_value is None:
            raise ComparisonError(f'{entry["id"]}: missing independent oracle argument')
        typed=simd_scaling.typed_input(entry)
        allowed=simd_scaling.allowed_wire_results(entry,simd_workloads)
        profile=simd_scaling.execution_profile(entry)
        fixtures.append({'entry':entry,'path':str(wasm),'wasm':wasm,'wat':wat,
                         'input':typed,'raw_input':raw_value,'allowed':allowed,
                         'profile':profile,'sha256':sha(wasm)})

    source_map=sources(include_simd=True)
    source_paths=[candidate,baseline_exe,encoder,HERE/'simd_workloads.py',HERE/'simd_scaling.py',
                  HERE/'workloads.py',HERE/'workloads.json',HERE/'baseline.py',protocol,
                  *[fixture['wat'] for fixture in fixtures],*[fixture['wasm'] for fixture in fixtures]]
    if growth_explanation is not None:
        source_paths.append(growth_explanation['path'])
    source_before={str(path.resolve()):sha(path) for path in sorted(set(source_paths))}
    initial_sources=source_map; initial_candidate=sha(candidate); initial_baseline=sha(baseline_exe)
    profiles=sorted({fixture['profile'] for fixture in fixtures})
    capabilities={key:process(exe,['--protocol'],cpu)
                  for key,exe in [('baseline',baseline_exe),('candidate',candidate)]}
    for key,cap in capabilities.items():
        mode=baseline_mode if key=='baseline' else args.candidate_mode
        _validate_simd_capability(key,cap,mode,profiles)
    metadata={'settings':{'instrumented':False,'profile':'release','baseline_mode':baseline_mode,
                          'candidate_mode':args.candidate_mode,'feature_policy':'per-workload',
                          'fuel_policy':False,'cancellation_policy':1,
                          'profiles':profiles,'warm_target_ns':20_000_000},
              'candidate_sources':initial_sources,
              'hardware':{'selected_cpu':cpu,'cpu_affinity_available':allowed_cpus,
                          'cpu_quota':Path('/sys/fs/cgroup/cpu.max').read_text().strip(),
                          'cpu_model':next((line.split(':',1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),'unknown'),
                          'physical_core_idle':'unknown/shared cloud'},
              'baseline_archive_kind':baseline_info['kind'],
              'baseline_archive_path':str(args.baseline.resolve()),
              'baseline_manifest_sha256':sha(args.baseline/'manifest.json'),
              'frozen_manifest_sha256':sha(frozen/'manifest.json'),
              'baseline_binary_sha256':initial_baseline,'candidate_binary_sha256':initial_candidate,
              'fixture_encoder_sha256':encoder_hash,
              'simd_model_sha256':sha(HERE/'simd_workloads.py'),
              'simd_registry_sha256':sha(HERE/'workloads.json'),
              'fixtures':[{'workload':fixture['entry']['id'],'wat_sha256':sha(fixture['wat']),
                           'wasm_sha256':fixture['sha256'],'profile':fixture['profile'],
                           'allowed_results':fixture['allowed']} for fixture in fixtures],
              'capabilities':capabilities}
    if growth_explanation is not None:
        metadata.update(growth_explanation['metadata'])
    report={'schema_version':2,'passed':False,'source_stable':False,
            'selection':args.selection,'metadata':metadata,'runs':[],
            'correctness_probes':[],'warm_calibrations':[]}
    identity={'selection':args.selection,'metadata':metadata,'pairs':args.pairs,'seed':args.seed}
    journal=checkpoint.Journal(args.output,identity,
                               {run:args.seed+run*1000003 for run in range(2)},prepared=True)
    try:
        sides=(('baseline',baseline_exe,baseline_mode),('candidate',candidate,args.candidate_mode))
        # Correctness is checked on both uninstrumented binaries before samples begin.
        for fixture in fixtures:
            probe_sides={}
            for side,exe,mode in sides:
                observed=_simd_observation(exe,fixture['path'],fixture['entry'],fixture['input'],
                    fixture['raw_input'],fixture['allowed'],'first_call',1,mode,fixture['profile'],
                    cpu,model=simd_workloads,expected_batch=1)
                probe_sides[side]={key:value for key,value in observed.items() if key!='ns_per_operation'}
            report['correctness_probes'].append({'kind':'typed_simd_oracle','workload':fixture['entry']['id'],
                'timing_excluded':True,'status':'passed','allowed_results':fixture['allowed'],
                'sides':probe_sides})

        for run_index in range(2):
            seed=args.seed+run_index*1000003
            pairs=[]; report['runs'].append({'run_id':run_index,'seed':seed,'pairs':pairs})
            for fixture in fixtures:
                entry=fixture['entry']; case=entry['id']
                for stage in entry['stages']:
                    batch=1
                    if stage=='warm_call':
                        calibrations=[]
                        for side,exe,mode in sides:
                            calibrations.append(_calibrate_simd(exe,fixture['path'],entry,
                                fixture['input'],fixture['raw_input'],fixture['allowed'],mode,
                                fixture['profile'],cpu,model=simd_workloads))
                        batch=max(row['batch'] for row in calibrations)
                        if not 1<=batch<=100000:
                            raise ComparisonError(f'{case}: paired SIMD warm batch outside 1..100000')
                        report['warm_calibrations'].append({'run_id':run_index,'workload':case,
                            'stage':stage,'target_ns':20_000_000,'paired_batch':batch,
                            'sides':[{'side':side,'batch':row['batch'],
                                      'calibrated_total_ns':row['batch']*row['ns_per_operation']}
                                     for (side,_exe,_mode),row in zip(sides,calibrations)]})
                    orders=pair_order(seed+int(fixture['sha256'][:8],16)+STAGES.index(stage),args.pairs)
                    for sample,order in enumerate(orders):
                        normalized=([int(value.removeprefix('i32:')) for value in fixture['allowed']])
                        expected=normalized[0] if len(normalized)==1 else {'allowed_results':normalized}
                        pair={'pair_id':f'{run_index}:{case}:{stage}:{sample}','workload':case,
                              'stage':stage,'input':fixture['input'],'expected':expected,
                              'wasm_sha256':fixture['sha256'],'order':order,
                              'profile':fixture['profile']}
                        for side in order:
                            _side,exe,mode=next(item for item in sides if item[0]==side)
                            pair[side]=_simd_observation(exe,fixture['path'],entry,fixture['input'],
                                fixture['raw_input'],fixture['allowed'],stage,batch,mode,
                                fixture['profile'],cpu,model=simd_workloads,expected_batch=batch)
                        if pair['baseline']['result']!=expected or pair['candidate']['result']!=expected:
                            raise ComparisonError(f'{case}/{stage}: normalized SIMD oracle mismatch')
                        pairs.append(pair); journal.append(run_index,seed,pair)
                    print(f'run {run_index+1} {case}/{stage}: {args.pairs} valid pairs',flush=True)
        source_after={str(path.resolve()):sha(path) for path in sorted(set(source_paths))}
        if (source_before!=source_after or initial_sources!=sources(include_simd=True)
                or initial_candidate!=sha(candidate) or initial_baseline!=sha(baseline_exe)
                or encoder_hash!=sha(encoder)):
            raise ComparisonError('SIMD source, binary, encoder, or fixture identity changed during measurement')
        _verify_build_growth_explanation(growth_explanation)
        baseline.verify_archive(args.baseline); baseline.verify_archive(frozen)
        report['passed']=True; report['source_stable']=True
    except Exception as exc:
        report['error']=str(exc); journal.write_report_once(report); raise
    finally:
        journal.close()
    journal.write_report_once(report)
    return report

def process(exe, args, cpu, timeout=180):
    result = subprocess.run([str(exe), *map(str,args)], text=True, capture_output=True,
                            timeout=timeout, preexec_fn=lambda:os.sched_setaffinity(0,{cpu}))
    if result.returncode:
        raise ComparisonError(f'process failed ({result.returncode}): {result.stderr[-4000:]}')
    try: data = json.loads(result.stdout)
    except (ValueError, TypeError) as exc: raise ComparisonError('missing or malformed process response') from exc
    if not isinstance(data,dict): raise ComparisonError('response must be an object')
    return data

def validate_observation(data, case, stage, input_value, expected, batch, mode):
    wanted = {'schema_version':2,'case':case,'stage':stage,'input':input_value,
              'expected':expected,'mode':mode}
    if any(data.get(k) != v for k,v in wanted.items()):
        raise ComparisonError('response identity/settings mismatch')
    ns = data.get('ns_per_operation'); count=data.get('batch')
    if isinstance(ns,bool) or not isinstance(ns,(int,float)) or not math.isfinite(ns) or ns <= 0:
        raise ComparisonError('invalid timing')
    if isinstance(count,bool) or not isinstance(count,int) or count < 1 or count > 100000:
        raise ComparisonError('invalid batch')
    if batch > 0 and count != batch: raise ComparisonError('batch mismatch')
    oracle = None if case == 'prepare_large' else expected
    if 'result' not in data or data['result'] != oracle: raise ComparisonError('oracle mismatch')
    return data

def observation(exe, fixture, case, input_value, expected, stage, batch, mode, cpu, fuel_setting=None):
    args=['--single-case',fixture,case,input_value,expected,stage,batch,mode]
    if fuel_setting is not None:
        import p127_fuel_policy
        args.extend(['--source-fuel',p127_fuel_policy.cli_value(fuel_setting)])
    result=validate_observation(process(exe,args,cpu),case,stage,input_value,expected,batch,mode)
    if fuel_setting is not None:
        p127_fuel_policy.validate_wire_setting(result,fuel_setting)
    return result

def _typed_i32(value):
    if type(value) is not int:
        raise ComparisonError('scalar oracle must return a signed i32 integer')
    if not -(1 << 31) <= value < (1 << 31):
        raise ComparisonError('scalar oracle result is outside signed i32 range')
    return f'i32:{value}'

def scalar_observation(exe, fixture, entry, input_value, expected, stage, batch, mode, cpu, fuel_setting=None):
    case=entry['id']; driver=entry['driver']['kind']; profile=entry['driver']['profile']
    argv=['--scalar-driver',fixture,case,input_value,expected,stage,batch,mode,profile,driver]
    if fuel_setting is not None:
        import p127_fuel_policy
        argv.extend(['--source-fuel',p127_fuel_policy.cli_value(fuel_setting)])
    data=process(exe,argv,cpu)
    if fuel_setting is not None:
        p127_fuel_policy.validate_wire_setting(data,fuel_setting)
    wanted={'schema_version':2,'case':case,'stage':stage,'input':_typed_i32(input_value),
            'expected':_typed_i32(expected),'mode':mode,'profile':profile,'driver':driver,
            }
    if any(data.get(key)!=value for key,value in wanted.items()):
        raise ComparisonError('scalar response identity/settings mismatch')
    if data.get('diagnostic') is not False:
        raise ComparisonError('scalar timing response must have diagnostic=false')
    if data.get('result') != _typed_i32(expected):
        raise ComparisonError('scalar oracle mismatch')
    ns=data.get('ns_per_operation'); count=data.get('batch')
    if isinstance(ns,bool) or not isinstance(ns,(int,float)) or not math.isfinite(ns) or ns<=0:
        raise ComparisonError('invalid scalar timing')
    if isinstance(count,bool) or not isinstance(count,int) or count<1 or count>100000:
        raise ComparisonError('invalid scalar batch')
    if batch>0 and count!=batch: raise ComparisonError('scalar batch mismatch')
    result=dict(data)
    result['input']=input_value; result['expected']=expected; result['result']=expected
    result['wire_observation']=data
    return result

def pair_order(seed, count):
    rng=random.Random(seed)
    return [['baseline','candidate'] if rng.getrandbits(1) else ['candidate','baseline'] for _ in range(count)]

def _baseline_identity(archive, manifest, protocol, baseline_tool):
    """Verify either a strict legacy adapter or a self-frozen current-source archive."""
    archive=Path(archive).resolve()
    protocol=Path(protocol).resolve()
    if manifest.get('kind') == 'frozen-typed-adapter':
        import p123_adapter
        verified = p123_adapter.verify(archive)
        return {'frozen':archive,'executable':archive/verified['binary'],
                'mode':verified['mode'],'protocol_sha256':verified['protocol_sha256'],
                'kind':p123_adapter.KIND}
    if manifest.get('kind') == 'legacy-api-adapter':
        frozen=Path(manifest.get('baseline','')).resolve()
        baseline_tool.verify_archive(frozen)
        verified=baseline_tool.verify_adapter(archive,frozen,protocol)
        if sha(frozen/'manifest.json') != verified.get('baseline_manifest_sha256'):
            raise ComparisonError('frozen baseline manifest changed')
        if sha(protocol) != verified.get('protocol_sha256'):
            raise ComparisonError('baseline adapter uses a different protocol; create a new adapter archive')
        executable=archive/verified.get('binary','')
        return {'frozen':frozen,'executable':executable,'mode':'decoded',
                'protocol_sha256':verified['protocol_sha256'],'kind':'legacy-api-adapter'}

    files=manifest.get('files',{})
    policy=manifest.get('execution_policy')
    if not isinstance(policy,dict):
        raise ComparisonError('current-source archive is missing execution_policy metadata')
    requested_mode=policy.get('requested_mode')
    baseline_mode=policy.get('resolved_mode')
    if not isinstance(requested_mode,str) or not requested_mode:
        raise ComparisonError('current-source archive is missing its requested execution mode')
    if not isinstance(baseline_mode,str) or not baseline_mode:
        raise ComparisonError('current-source archive is missing its resolved execution mode')
    if requested_mode not in (*baseline_tool.MODE_FEATURES,'portable'):
        raise ComparisonError('current-source archive requested an unknown execution mode')
    expected_resolved='decoded' if requested_mode=='portable' else requested_mode
    if baseline_mode != expected_resolved:
        raise ComparisonError('current-source archive requested/resolved execution modes disagree')
    if manifest.get('mode') != baseline_mode:
        raise ComparisonError('current-source archive mode metadata is inconsistent')
    recorded_modes=policy.get('protocol_modes')
    if not isinstance(recorded_modes,list) or baseline_mode not in recorded_modes:
        raise ComparisonError('current-source archive mode is absent from recorded protocol modes')

    protocol_rel='source/tools/wtime-bench/src/protocol.rs'
    protocol_record=files.get(protocol_rel)
    if (not isinstance(protocol_record,dict) or protocol_record.get('role')!='source'
            or protocol_record.get('source_path')!='tools/wtime-bench/src/protocol.rs'):
        raise ComparisonError('current-source archive is missing its archived measurement protocol')
    archived_protocol=archive/protocol_rel
    protocol_hash=sha(archived_protocol)
    if protocol_record.get('sha256') != protocol_hash or policy.get('protocol_sha256') != protocol_hash:
        raise ComparisonError('current-source archive protocol identity mismatch')
    if sha(protocol) != protocol_hash:
        raise ComparisonError('current-source archive uses a different protocol; create a fresh archive')

    binary_rel='binary/wtime-bench'
    binary_record=files.get(binary_rel)
    if (not isinstance(binary_record,dict)
            or binary_record.get('role')!='release_benchmark_executable'):
        raise ComparisonError('current-source archive is missing its benchmark binary identity')
    executable=archive/binary_rel
    if not executable.is_file() or not os.access(executable,os.X_OK):
        raise ComparisonError('current-source archive benchmark binary is missing or not executable')
    binary_hash=sha(executable)
    if binary_record.get('sha256') != binary_hash:
        raise ComparisonError('current-source archive benchmark binary identity mismatch')
    smoke=manifest.get('mode_smoke')
    if (not isinstance(smoke,dict) or smoke.get('mode')!=baseline_mode
            or smoke.get('binary_sha256')!=binary_hash):
        raise ComparisonError('current-source archive mode smoke does not bind the selected mode and binary')
    return {'frozen':archive,'executable':executable,'mode':baseline_mode,
            'protocol_sha256':protocol_hash,'kind':'current-source-archive'}

def _verify_scalar_harness_sources(archive, manifest):
    """Require the current-source baseline to bind every live protocol leaf."""
    if manifest.get('kind') == 'frozen-typed-adapter':
        import p123_adapter
        p123_adapter.verify(archive)
        return
    files=manifest.get('files',{})
    live_sources=sorted((HERE/'src').glob('*.rs'))
    live_keys={'source/'+source.relative_to(ROOT).as_posix() for source in live_sources}
    archived_keys={key for key in files
                   if key.startswith('source/tools/wtime-bench/src/') and key.endswith('.rs')}
    if archived_keys!=live_keys:
        raise ComparisonError('current-source archive harness leaf set differs from this comparison harness')
    for source in live_sources:
        relative=Path('source')/source.relative_to(ROOT)
        key=relative.as_posix()
        record=files.get(key)
        if not isinstance(record,dict) or record.get('role')!='source':
            raise ComparisonError(f'current-source archive is missing harness leaf {source.name}')
        if record.get('source_path') != source.relative_to(ROOT).as_posix():
            raise ComparisonError(f'current-source archive harness leaf path mismatch: {source.name}')
        archived=Path(archive)/relative
        if not archived.is_file() or record.get('sha256')!=sha(archived):
            raise ComparisonError(f'current-source archive harness leaf identity mismatch: {source.name}')
        if sha(source)!=sha(archived):
            raise ComparisonError(f'current-source archive uses a different harness leaf: {source.name}')

def _verify_memory_harness_sources(archive, manifest):
    """Memory CLI needs every archived harness leaf to match this source tree."""
    if manifest.get('kind') == 'legacy-api-adapter':
        raise ComparisonError('P009 memory selection is unsupported by the legacy-api baseline adapter')
    _verify_scalar_harness_sources(archive, manifest)
    if manifest.get('kind') == 'frozen-typed-adapter':
        return
    files=manifest.get('files',{})
    required=(HERE/'src'/'memory_driver.rs').relative_to(ROOT).as_posix()
    key='source/'+required
    record=files.get(key)
    archived=Path(archive)/key
    if (not isinstance(record,dict) or record.get('role')!='source'
            or record.get('source_path')!=required or not archived.is_file()
            or record.get('sha256')!=sha(archived) or sha(archived)!=sha(ROOT/required)):
        raise ComparisonError('current-source archive does not bind the live memory driver leaf')

def _validate_memory_stage_contract(entry):
    family=entry.get('family')
    stages=entry.get('stages')
    expected=STAGES if family in MEMORY_STAGED_FAMILIES else ['load_and_call']
    if stages != expected:
        raise ComparisonError(f"{entry.get('id')}: memory stage contract must be {expected}")

def _validate_memory_model_rows(selected_entries, modeled_entries):
    by_id={entry.get('id'):entry for entry in modeled_entries}
    selected_by_id={entry.get('id'):entry for entry in selected_entries}
    if len(by_id)!=len(modeled_entries) or set(by_id)!=set(selected_by_id):
        raise ComparisonError('memory selection does not cover the complete memory_workloads model')
    for case,entry in by_id.items():
        if selected_by_id[case]!=entry:
            raise ComparisonError(f'{case}: full memory registry row differs from independent model')

def _memory_wire_outcome(expected, memory_module):
    import memory_scaling
    outcome=memory_scaling._expected_outcome(expected)
    return outcome if outcome=='trap' else int(outcome.removeprefix('i32:'))

def memory_observation(exe, fixture, entry, input_value, expected, stage, batch,
                       mode, cpu, *, memory_module):
    import memory_scaling
    profile=memory_scaling._profile(entry)
    driver=memory_scaling._driver(entry)
    outcome=memory_scaling._expected_outcome(expected)
    args=['--memory-staged',fixture,entry['id'],f'i32:{input_value}',outcome,
          stage,batch,mode,profile,driver]
    data=process(exe,args,cpu)
    verified=memory_scaling._validate_observation(
        data,entry,input_value,expected,mode,profile,driver,
        stage=stage,expected_batch=None if int(batch)==0 else int(batch))
    actual_batch=verified['batch']
    if stage!='warm_call' and actual_batch!=1:
        raise ComparisonError(f'{entry["id"]}/{stage}: cold stage must run exactly one call')
    normalized_outcome=_memory_wire_outcome(expected,memory_module)
    wire={key:value for key,value in data.items() if key not in {'memory_hex','all_memory_hex'}}
    return {'input':input_value,'expected':normalized_outcome,'result':normalized_outcome,
            'ns_per_operation':float(data['ns_per_operation']),'batch':actual_batch,
            'mode':mode,'profile':profile,'driver':driver,
            'memory':verified,'wire_observation':wire}

def memory_boundary_probe(exe, fixture, entry, expected, mode, cpu, model):
    """Validate the untimed memory64 edge with the selected executor policy."""
    import memory_scaling
    args = memory_scaling._memory_driver_args(fixture, entry, model.PAGE,
        memory_scaling._expected_outcome(expected), 'all-phase3-core', 'plain', mode=mode)
    raw = process(exe, args, cpu)
    verified = memory_scaling._validate_observation(raw, entry, model.PAGE,
        expected, mode, 'all-phase3-core', 'plain')
    wire = {key: value for key, value in raw.items()
            if key not in {'memory_hex', 'all_memory_hex', 'ns_per_operation'}}
    return verified, wire

def _is_calibratable_scalar_fixture(fixture):
    """Allow typed rows and only the three explicit legacy selection controls."""
    if fixture.get('scalar') is True:
        return fixture.get('memory') is not True
    return (fixture.get('id') in LEGACY_SCALAR_CONTROL_IDS
            and fixture.get('scalar') is False and fixture.get('memory') is False)


def _calibrate_scalar_shared_batch(fixture, sides, cpu, *, target_ns=20_000_000,
                                   max_batch=100000, max_attempts=8):
    """Find one exact warm batch whose measured interval clears target on both sides."""
    if not _is_calibratable_scalar_fixture(fixture):
        raise ComparisonError('shared scalar calibration requires a typed scalar fixture or an explicit legacy control')
    if (type(target_ns) is not int or target_ns <= 0 or type(max_batch) is not int
            or not 1 <= max_batch <= 100000 or type(max_attempts) is not int
            or not 1 <= max_attempts <= 8):
        raise ComparisonError('invalid scalar calibration limits')
    if len(sides) != 2:
        raise ComparisonError('scalar calibration requires baseline and candidate sides')

    def observe(side, requested):
        name, exe, mode = side
        row = _observe(exe, fixture, 'warm_call', requested, mode, cpu)
        actual = row.get('batch')
        ns = row.get('ns_per_operation')
        if (isinstance(actual, bool) or not isinstance(actual, int)
                or not 1 <= actual <= max_batch):
            raise ComparisonError(f"{fixture['id']}: invalid scalar calibration batch")
        if requested and actual != requested:
            raise ComparisonError(f"{fixture['id']}: scalar calibration batch differs from request")
        if (isinstance(ns, bool) or not isinstance(ns, (int, float))
                or not math.isfinite(ns) or ns <= 0):
            raise ComparisonError(f"{fixture['id']}: scalar calibration timing must be finite and positive")
        total = actual * float(ns)
        if not math.isfinite(total) or total <= 0:
            raise ComparisonError(f"{fixture['id']}: scalar calibrated interval must be finite and positive")
        return {'side': name, 'requested_batch': requested, 'observed_batch': actual,
                'ns_per_operation': float(ns), 'calibrated_total_ns': total}

    initial = [observe(side, 0) for side in sides]
    batch = max(row['observed_batch'] for row in initial)
    attempts = []
    confirmations = []
    for attempt_number in range(1, max_attempts + 1):
        confirmations = [observe(side, batch) for side in sides]
        attempts.append({'attempt': attempt_number, 'batch': batch,
                         'sides': confirmations})
        if all(row['calibrated_total_ns'] >= target_ns for row in confirmations):
            return {'batch': batch, 'initial_estimates': initial,
                    'attempts': attempts, 'sides': confirmations,
                    'target_ns': target_ns, 'max_batch': max_batch,
                    'max_attempts': max_attempts}
        if batch >= max_batch:
            break
        fastest_ns = min(row['ns_per_operation'] for row in confirmations)
        # Saturate before division/ceil: very small positive values can make
        # the projected duration infinite even though each wire timing is finite.
        saturation_threshold = target_ns * 1.1 / max_batch
        grown = (max_batch if fastest_ns <= saturation_threshold else
                 math.ceil(target_ns / fastest_ns * 1.1))
        batch = min(max_batch, max(batch + 1, grown))
    audit = {'target_ns': target_ns, 'max_batch': max_batch,
             'max_attempts': max_attempts, 'initial_estimates': initial,
             'attempts': attempts}
    raise ComparisonError(f"{fixture['id']}: scalar warm calibration did not reach {target_ns}ns "
                          f"on both sides within {max_batch} calls and {max_attempts} attempts; "
                          f"calibration_audit={json.dumps(audit, sort_keys=True)}")


def _calibrate_memory(exe, fixture, mode, cpu, *, target_ns=20_000_000,
                      max_batch=100000, max_attempts=8):
    """Adapt memory warm-call batch until validated observations reach target."""
    requested=0
    for _attempt in range(max_attempts):
        row=_observe(exe,fixture,'warm_call',requested,mode,cpu)
        actual=row.get('batch'); ns=row.get('ns_per_operation')
        if isinstance(actual,bool) or not isinstance(actual,int) or not 1<=actual<=max_batch:
            raise ComparisonError(f'{fixture["id"]}: invalid memory calibration batch')
        if requested and actual!=requested:
            raise ComparisonError(f'{fixture["id"]}: memory calibration response batch differs from request')
        if isinstance(ns,bool) or not isinstance(ns,(int,float)) or not math.isfinite(ns) or ns<=0:
            raise ComparisonError(f'{fixture["id"]}: memory calibration timing must be finite and positive')
        if actual*ns>=target_ns:
            return row
        if actual>=max_batch:
            break
        requested=min(max_batch,max(actual+1,math.ceil(target_ns/ns*1.1)))
    raise ComparisonError(f'{fixture["id"]}: memory warm calibration did not reach {target_ns}ns '
                          f'within {max_batch} calls and {max_attempts} attempts')


def _encode_scalar_fixture(entry, output_dir, encoder):
    source=HERE/entry['source']['path']
    if not source.is_file() or sha(source)!=entry['source']['sha256']:
        raise ComparisonError(f"{entry['id']}: scalar WAT source hash mismatch")
    output=output_dir/(Path(entry['source']['path']).stem+'.wasm')
    if not output.exists():
        result=subprocess.run([str(encoder),'encode',str(source),'-o',str(output)],cwd=ROOT,
                              text=True,capture_output=True,timeout=60)
        if result.returncode:
            raise ComparisonError(f"{entry['id']}: fixture encoding failed: {result.stderr[-2000:]}")
    if sha(output)!=entry['source']['wasm_sha256']:
        raise ComparisonError(f"{entry['id']}: encoded scalar Wasm hash mismatch")
    return output

def _observe(exe, fixture, stage, batch, mode, cpu):
    if fixture.get('scalar'):
        return scalar_observation(exe,fixture['path'],fixture['entry'],fixture['input'],
                                  fixture['expected'],stage,batch,mode,cpu)
    if fixture.get('memory'):
        return memory_observation(exe,fixture['path'],fixture['entry'],fixture['input'],
                                  fixture['expected_memory'],stage,batch,mode,cpu,
                                  memory_module=fixture['memory_module'])
    return observation(exe,fixture['path'],fixture['id'],fixture['input'],
                       fixture['expected'],stage,batch,mode,cpu)

def _probe_json(exe, args, cpu, *, case, driver, mode, profile, status='trap_verified'):
    data=process(exe,args,cpu)
    if (data.get('schema_version')!=2 or data.get('case')!=case
            or data.get('driver')!=driver or data.get('mode')!=mode
            or data.get('profile')!=profile or data.get('diagnostic') is not True
            or data.get('status')!=status or 'ns_per_operation' in data):
        raise ComparisonError(f'{case}: scalar correctness probe identity/status mismatch')
    return data

def _preflight_scalar_side(exe, mode, entries, fixture_by_id, cpu, scalar_module):
    by_family={}
    for entry in entries:
        by_family.setdefault(entry['family'],[]).append(entry)
    table=by_family.get('scalar_table_mutation')
    if table:
        entry=next((item for item in table if item['params'][0]['baseline']==17),table[0])
        n=entry['params'][0]['baseline']
        expected_a,expected_b=scalar_module.table_mutation_probe(n)
        path=fixture_by_id[entry['id']]
        data=process(exe,['--scalar-table-probe',path,n,expected_a,expected_b,mode,
                          entry['driver']['profile']],cpu)
        if (data.get('case')!='scalar_table_mutation' or data.get('driver')!='table_mutation'
                or data.get('diagnostic') is not True or data.get('status')!='mutation_verified'
                or data.get('input')!=f'i32:{n}' or data.get('target_a')!=_typed_i32(expected_a)
                or data.get('target_b')!=_typed_i32(expected_b)
                or data.get('target_a_after_restore')!=_typed_i32(expected_a)
                or data.get('mode')!=mode or data.get('profile')!=entry['driver']['profile']
                or 'ns_per_operation' in data):
            raise ComparisonError('scalar table mutation probe mismatch')
        for driver,trap_kind in (('probe_null','NullReference'),
                                 ('probe_mismatch','IndirectCallTypeMismatch')):
            probe=_probe_json(exe,['--scalar-driver',path,entry['id'],0,0,'first_call',1,mode,
                                   entry['driver']['profile'],driver],cpu,case=entry['id'],
                              driver=driver,mode=mode,profile=entry['driver']['profile'])
            if probe.get('trap_kind')!=trap_kind:
                raise ComparisonError(f'scalar {driver} probe reported the wrong trap kind')
    recursion=by_family.get('scalar_ordinary_recursion')
    if recursion:
        entry=next((item for item in recursion if item['params'][0]['baseline']==32),recursion[0])
        path=fixture_by_id[entry['id']]
        for n in (1024,10000):
            probe=_probe_json(exe,['--scalar-driver',path,entry['id'],n,0,'first_call',1,mode,
                                   entry['driver']['profile'],'depth_probe'],cpu,case=entry['id'],
                              driver='depth_probe',mode=mode,profile=entry['driver']['profile'])
            if probe.get('trap_kind')!='ResourceLimit' or 'call depth' not in probe.get('message','').lower():
                raise ComparisonError('ordinary recursion probe did not hit the call-depth limit')
    tail=by_family.get('scalar_tail_calls')
    if tail:
        entry=next((item for item in tail if item['params'][0]['baseline']==10000),tail[0])
        n=10000; expected=scalar_module.oracle(entry['id'],n); path=fixture_by_id[entry['id']]
        probe=_probe_json(exe,['--scalar-driver',path,entry['id'],n,expected,'first_call',1,mode,
                               entry['driver']['profile'],'tail_probe'],cpu,case=entry['id'],
                          driver='tail_probe',mode=mode,profile=entry['driver']['profile'],
                          status='returned')
        if (probe.get('input')!=_typed_i32(n)
                or probe.get('expected')!=_typed_i32(expected)
                or probe.get('result')!=_typed_i32(expected)):
            raise ComparisonError('tail-call depth probe result mismatch')

def resolve_compute_subset(selected, subset):
    if subset is None:
        return selected
    if len(subset)!=3 or len(set(subset))!=3 or set(subset)!=LEGACY_SCALAR_CONTROL_IDS:
        raise ComparisonError('P001 compute subset must be exactly call_overhead/integer_loop/branch_calls')
    if not set(subset)<={entry['id'] for entry in selected}:
        raise ComparisonError('subset IDs absent from actual registry selection')
    return [entry for entry in selected if entry['id'] in set(subset)]


def run(args):
    growth_explanation=_read_build_growth_explanation(
        getattr(args,'build_growth_explanation_file',None))
    if getattr(args, 'workload_subset', None) and args.selection != 'original-mvp':
        raise ComparisonError('explicit workload subset is restricted to original-mvp legacy controls')
    if args.selection == 'representative':
        import representative_compare
        try:
            return representative_compare.run(args, growth_explanation)
        except representative_compare.RepresentativeComparisonError as exc:
            raise ComparisonError(str(exc)) from exc
    if args.selection == 'integration':
        import integration_compare
        return integration_compare.run(args, growth_explanation)
    if args.selection == 'simd':
        return _run_simd(args, growth_explanation)
    if args.selection in {'gc-exceptions', 'gc-layouts'}:
        import gc_compare
        try:
            return gc_compare.run(args, growth_explanation)
        except gc_compare.GcComparisonError as exc:
            raise ComparisonError(str(exc)) from exc
    import checkpoint
    checkpoint.Journal.preflight(args.output)
    # Lazy imports keep validation helpers independently testable.
    import baseline
    import run as original
    import workloads
    import scalar_workloads

    manifest=baseline.verify_archive(args.baseline)
    protocol=HERE/'src/protocol.rs'
    baseline_info=_baseline_identity(args.baseline,manifest,protocol,baseline)
    frozen=baseline_info['frozen']
    baseline_exe=baseline_info['executable']
    baseline_mode=baseline_info['mode']
    selected=workloads.selection(args.selection)
    selected=resolve_compute_subset(selected,getattr(args,'workload_subset',None))

    scalar_entries=[entry for entry in selected
                    if str(entry.get('family','')).startswith('scalar_')]
    memory_entries=[entry for entry in selected
                    if entry.get('id')!='memory_fill_scan'
                    and entry.get('oracle', {}).get('module') == 'memory_workloads.py']
    memory_workloads=None
    if memory_entries:
        import memory_workloads as memory_workloads_module
        memory_workloads=memory_workloads_module
        modeled=memory_workloads.entries()
        _validate_memory_model_rows(memory_entries,modeled)
        if args.selection=='memory':
            legacy_rows=[entry for entry in selected if entry.get('id')=='memory_fill_scan']
            if len(legacy_rows)!=1:
                raise ComparisonError('memory selection must retain the original memory_fill_scan row exactly once')
        for entry in memory_entries:
            _validate_memory_stage_contract(entry)
        if baseline_info['kind']=='legacy-api-adapter':
            raise ComparisonError('P009 memory selection is unsupported by the legacy-api baseline adapter')
        _verify_memory_harness_sources(frozen,manifest)
    if scalar_entries and baseline_info['kind']=='legacy-api-adapter':
        raise ComparisonError('P008 scalar selection is unsupported by the legacy-api baseline adapter')
    if scalar_entries:
        _verify_scalar_harness_sources(frozen,manifest)

    candidate=args.candidate.resolve()
    initial_binary=sha(candidate); initial_sources=sources(include_memory=bool(memory_entries))
    original_binary=sha(baseline_exe)
    allowed=sorted(os.sched_getaffinity(0)); cpu=args.cpu if args.cpu is not None else allowed[0]
    if cpu not in allowed: raise ComparisonError('CPU outside available affinity')
    args.output.mkdir(parents=True,exist_ok=False)
    scalar_fixture_dir=args.output/'fixtures'/'scalar'
    scalar_fixture_by_id={}
    if scalar_entries:
        encoder=ROOT/'target'/'debug'/'wtime-conformance'
        if not encoder.is_file() or not os.access(encoder,os.X_OK):
            raise ComparisonError('existing wtime-conformance encoder is missing or not executable')
        scalar_fixture_dir.mkdir(parents=True,exist_ok=True)
        for entry in scalar_entries:
            scalar_fixture_by_id[entry['id']]=_encode_scalar_fixture(entry,scalar_fixture_dir,encoder)

    memory_fixture_by_name={}
    memory_fixture_dir=args.output/'fixtures'/'memory'
    encoder=ROOT/'target'/'debug'/'wtime-conformance'
    encoder_hash_before=None
    if memory_entries:
        import memory_scaling
        if not encoder.is_file() or not os.access(encoder,os.X_OK):
            raise ComparisonError('existing wtime-conformance encoder is missing or not executable')
        encoder_hash_before=sha(encoder)
        try:
            memory_fixture_by_name=memory_scaling._encode(
                encoder,HERE,args.output,memory_workloads,cpu)
        except Exception as exc:
            raise ComparisonError(f'memory fixture encoding failed: {exc}') from exc
        for entry in memory_entries:
            name=Path(entry['source']['path']).name
            record=memory_fixture_by_name.get(name)
            if (record is None or record.get('wat_sha256')!=entry['source']['sha256']
                    or record.get('wasm_sha256')!=entry['source']['wasm_sha256']):
                raise ComparisonError(f"{entry['id']}: encoded memory fixture provenance mismatch")

    capabilities={key:process(exe,['--protocol'],cpu)
                  for key,exe in [('baseline',baseline_exe),('candidate',candidate)]}
    for key,cap in capabilities.items():
        if cap.get('schema_version')!=2 or cap.get('instrumented') is not False:
            raise ComparisonError(f'{key}: incompatible/instrumented harness')
        mode=baseline_mode if key=='baseline' else args.candidate_mode
        if mode not in cap.get('modes',[]): raise ComparisonError(f'{key}: mode {mode} unavailable')
        if scalar_entries:
            profiles={entry['driver']['profile'] for entry in scalar_entries}
            missing=profiles-set(cap.get('features',[]))
            if missing: raise ComparisonError(f'{key}: feature profiles unavailable: {sorted(missing)}')
        if memory_entries and 'all-phase3-core' not in cap.get('features',[]):
            raise ComparisonError(f'{key}: all-phase3-core is unavailable for the memory driver')

    fixtures=[]
    for entry in selected:
        case=entry['id']; input_value=entry['params'][0]['baseline'] if entry['params'] else 0
        is_scalar=entry in scalar_entries
        if is_scalar:
            driver=entry.get('driver')
            if (not isinstance(driver,dict) or driver.get('kind') not in
                    {'plain','host_tick','table_a','table_b'}
                    or driver.get('profile') not in {'mvp','stable-core','all-phase3-core'}):
                raise ComparisonError(f'{case}: scalar driver/profile is missing or invalid')
            fixture_path=scalar_fixture_by_id[case]
            expected=scalar_workloads.oracle(case,input_value)
            source_hash=entry['source']['sha256']
            fixture_data=dict(id=case,path=str(fixture_path),sha256=sha(fixture_path),
                              source_sha256=source_hash,input=input_value,expected=expected,
                              stages=entry['stages'],required_features=entry['required_features'],
                              profile=driver['profile'],driver=driver['kind'],family=entry['family'],
                              scalar=True,entry=entry)
        elif entry in memory_entries:
            import memory_scaling
            name=Path(entry['source']['path']).name
            fixture_path=args.output/'fixtures'/'memory'/Path(name).with_suffix('.wasm')
            record=memory_fixture_by_name[name]
            if (sha(fixture_path)!=entry['source']['wasm_sha256']
                    or record['wasm_sha256']!=entry['source']['wasm_sha256']):
                raise ComparisonError(f'{case}: encoded memory fixture hash mismatch')
            expected_memory=memory_workloads.expected_memory(case,input_value)
            expected=memory_workloads.oracle(case,input_value)
            expected_outcome=memory_scaling._expected_outcome(expected_memory)
            if ((expected_outcome=='trap' and expected!='trap') or
                    (expected_outcome!='trap' and expected!=int(expected_outcome.removeprefix('i32:')))):
                raise ComparisonError(f'{case}: scalar and byte-array memory oracles disagree')
            fixture_data=dict(id=case,path=str(fixture_path),sha256=record['wasm_sha256'],
                              source_sha256=record['wat_sha256'],input=input_value,
                              expected=expected,expected_memory=expected_memory,
                              stages=entry['stages'],required_features=entry['required_features'],
                              profile=memory_scaling._profile(entry),driver=memory_scaling._driver(entry),
                              family=entry['family'],memory=True,memory_module=memory_workloads,
                              entry=entry)
        else:
            fixture_path=frozen/'fixtures'/f'{case}.wasm'
            if sha(fixture_path)!=entry['source']['wasm_sha256']:
                raise ComparisonError('registry/frozen fixture hash mismatch')
            expected=original.oracle(case,input_value) if case!='prepare_large' else 0
            fixture_data=dict(id=case,path=str(fixture_path),sha256=sha(fixture_path),
                              source_sha256=entry['source']['sha256'],
                              input=input_value,expected=expected,stages=entry['stages'],
                              required_features=entry['required_features'],scalar=False,memory=False)
        fixtures.append(fixture_data)

    feature_profiles=sorted({fixture['profile'] for fixture in fixtures
                             if fixture.get('scalar') or fixture.get('memory')})
    feature_setting='mvp' if not feature_profiles or feature_profiles==['mvp'] else 'per-workload'
    report_fixtures=[]
    for fixture in fixtures:
        excluded={'entry','scalar','memory_module','expected_memory','memory'}
        reported={key:value for key,value in fixture.items() if key not in excluded}
        if fixture.get('memory'):
            expected_memory=fixture['expected_memory']
            reported['memory_oracle_summary']={key:value for key,value in expected_memory.items()
                                                if key not in {'bytes_hex','range'}}
            reported['memory_oracle_summary']['range']=expected_memory.get('range')
        report_fixtures.append(reported)
    hardware=dict(cpu_model=next((l.split(':',1)[1].strip() for l in Path('/proc/cpuinfo').read_text().splitlines() if l.startswith('model name')),'unknown'),
                  os=platform.platform(),cpu_affinity_available=allowed,selected_cpu=cpu,
                  cpu_quota=Path('/sys/fs/cgroup/cpu.max').read_text().strip(),physical_core_idle='unknown/shared cloud')
    selected_legacy_ids=sorted(entry['id'] for entry in selected
                                if entry.get('id') in LEGACY_SCALAR_CONTROL_IDS
                                and entry.get('oracle', {}).get('module') == 'run.py')
    settings=dict(features=feature_setting,feature_profiles=feature_profiles,
                   fuel=False,cancellation_quantum=1,profile='release',
                   instrumented=False,baseline_mode=baseline_mode,
                   candidate_mode=args.candidate_mode,
                   memory_profiles=sorted({'all-phase3-core'} if memory_entries else set()),
                   memory_common_batch=('max_of_calibrated_bounded_batches'
                                        if memory_entries else None),
                   memory_warm_calibration_target_ns=(20_000_000 if memory_entries else None))
    if scalar_entries or selected_legacy_ids:
        settings['scalar_warm_calibration']={
            'target_ns':20_000_000,'max_batch':100000,'max_attempts':8,
            'policy':'shared_exact_batch_both_sides_confirmed',
            'scope':'selected typed scalar-driver rows and selected explicit legacy controls',
            'typed_scalar_row_count':len(scalar_entries),
            'legacy_control_ids':selected_legacy_ids,
            'legacy_control_count':len(selected_legacy_ids),
            'legacy_scalar_rows':'shared exact-batch calibration through --single-case; ordinary integer JSON oracle'}
    report=dict(schema_version=2,passed=False,selection=args.selection,
                recorded_at_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                workload_subset=getattr(args,'workload_subset',None),
                metadata=dict(hardware=hardware,capabilities=capabilities,
                              settings=settings,
                              baseline_archive_kind=baseline_info['kind'],
                              baseline_archive_path=str(args.baseline.resolve()),
                              baseline_manifest_sha256=sha(args.baseline/'manifest.json'),
                              frozen_manifest_sha256=sha(frozen/'manifest.json'),
                              baseline_binary_sha256=original_binary,candidate_binary_sha256=initial_binary,
                              candidate_sources=initial_sources,protocol_sha256=sha(protocol),
                              fixture_encoder_sha256=sha(ROOT/'target'/'debug'/'wtime-conformance') if scalar_entries or memory_entries else None,
                              fixtures=report_fixtures,
                              rustc=subprocess.check_output(['rustc','-Vv'],text=True)),runs=[],
                correctness_probes=[],warm_calibrations=[])
    if growth_explanation is not None:
        report['metadata'].update(growth_explanation['metadata'])
    report_path=args.output/'report.json'
    identity = {'selection': args.selection, 'metadata': report['metadata'],
                'pairs': args.pairs, 'seed': args.seed}
    journal = checkpoint.Journal(args.output, identity,
        {run: args.seed + run * 1000003 for run in range(2)}, prepared=True)
    try:
        if scalar_entries:
            for exe,mode in ((baseline_exe,baseline_mode),(candidate,args.candidate_mode)):
                _preflight_scalar_side(exe,mode,scalar_entries,scalar_fixture_by_id,cpu,scalar_workloads)
        if memory_entries:
            import memory_scaling
            boundary_entry=next((entry for entry in memory_entries
                                 if entry['family']=='memory64_i32_extended'),None)
            if boundary_entry is None:
                raise ComparisonError('memory64 boundary correctness fixture is missing')
            boundary_fixture=next(fixture for fixture in fixtures
                                  if fixture.get('memory') and fixture['id']==boundary_entry['id'])
            boundary_expected=memory_workloads.expected_memory(boundary_entry['id'],memory_workloads.PAGE)
            if memory_workloads.oracle(boundary_entry['id'],memory_workloads.PAGE)!='trap':
                raise ComparisonError('memory64 boundary oracle did not predict an out-of-bounds trap')
            for side,exe,mode in (('baseline',baseline_exe,baseline_mode),
                                  ('candidate',candidate,args.candidate_mode)):
                boundary_verified,boundary_wire=memory_boundary_probe(
                    exe,boundary_fixture['path'],boundary_entry,boundary_expected,
                    mode,cpu,memory_workloads)
                report['correctness_probes'].append({
                    'kind':'memory64_i32_extended_page_boundary','side':side,
                    'case':boundary_entry['id'],'input':memory_workloads.PAGE,
                    'expected':'trap','timing_excluded':True,'status':'passed',
                    'address_extension':'i32.extend_i32_u','memory':boundary_verified,
                    'wire_observation':boundary_wire})
            # Correctness preflight is complete on both executables before the
            # first paired timing observation. Trap rows preserve byte effects.
            for fixture in fixtures:
                if not fixture.get('memory'):
                    continue
                preflight_sides={}
                for side,exe,mode in (('baseline',baseline_exe,baseline_mode),
                                      ('candidate',candidate,args.candidate_mode)):
                    observed=_observe(exe,fixture,'load_and_call',1,mode,cpu)
                    preflight_sides[side]={key:value for key,value in observed.items()
                                           if key!='ns_per_operation'}
                entry=fixture['entry']
                artifact={'kind':'memory_correctness_preflight','case':fixture['id'],
                          'family':fixture['family'],'expected':fixture['expected'],
                          'timing_excluded':True,'status':'passed','sides':preflight_sides}
                expected_memory=fixture['expected_memory']
                for key in ('preserved_prior_store','suppressed_later_store','effective_address',
                            'full_range_precheck'):
                    if key in expected_memory:
                        artifact[key]=expected_memory[key]
                report['correctness_probes'].append(artifact)
        for run_number in range(2):
            seed=args.seed+run_number*1000003
            pairs=[]; run_report={'run_id':run_number,'seed':seed,'pairs':pairs}
            report['runs'].append(run_report)
            for fixture in fixtures:
                case=fixture['id']; path=fixture['path']; input_value=fixture['input']; expected=fixture['expected']
                for stage in fixture['stages']:
                    batch=1
                    if stage=='warm_call':
                        if fixture.get('memory'):
                            calibration_rows=[_calibrate_memory(exe,fixture,mode,cpu)
                                              for exe,mode in [(baseline_exe,baseline_mode),
                                                               (candidate,args.candidate_mode)]]
                            calibrated=[row['batch'] for row in calibration_rows]
                            batch=_common_calibrated_batch(fixture,calibration_rows)
                        elif _is_calibratable_scalar_fixture(fixture):
                            calibration=_calibrate_scalar_shared_batch(
                                fixture, [('baseline',baseline_exe,baseline_mode),
                                          ('candidate',candidate,args.candidate_mode)], cpu)
                            batch=calibration['batch']
                            report['warm_calibrations'].append({
                                'run_id':run_number,'workload':case,'stage':stage,
                                'requested_batch':0,'minimum_calibration_ns':calibration['target_ns'],
                                'max_batch':calibration['max_batch'],
                                'max_attempts':calibration['max_attempts'],
                                'policy':'shared_exact_batch_both_sides_confirmed',
                                'workload_kind':('typed_scalar_driver' if fixture.get('scalar')
                                                 else 'legacy_single_case_control'),
                                'initial_estimates':calibration['initial_estimates'],
                                'attempts':calibration['attempts'],
                                'sides':calibration['sides'],'paired_batch':batch})
                        else:
                            calibration_rows=[_observe(exe,fixture,stage,0,mode,cpu)
                                              for exe,mode in [(baseline_exe,baseline_mode),
                                                               (candidate,args.candidate_mode)]]
                            calibrated=[row['batch'] for row in calibration_rows]
                            batch=_common_calibrated_batch(fixture,calibration_rows)
                        if fixture.get('memory'):
                            if batch>100000 or batch<1:
                                raise ComparisonError(f'{case}: calibrated warm batch is outside 1..100000')
                            report['warm_calibrations'].append({
                                'run_id':run_number,'workload':case,'stage':stage,
                                'requested_batch':0,'minimum_calibration_ns':20_000_000,
                                'sides':[{'side':side,'observed_batch':row['batch'],
                                          'ns_per_operation':row['ns_per_operation'],
                                          'calibrated_total_ns':row['batch']*row['ns_per_operation']}
                                         for side,row in zip(('baseline','candidate'),calibration_rows)],
                                'paired_batch':batch})
                    orders=pair_order(seed+int(fixture['sha256'][:8],16)+STAGES.index(stage),args.pairs)
                    for sample,order in enumerate(orders):
                        pair=dict(pair_id=f'{run_number}:{case}:{stage}:{sample}',workload=case,
                                  stage=stage,input=input_value,
                                  expected=None if case=='prepare_large' else expected,
                                  wasm_sha256=fixture['sha256'],order=order)
                        if fixture.get('scalar'):
                            pair.update(profile=fixture['profile'],driver=fixture['driver'],
                                        source_sha256=fixture['source_sha256'])
                        if fixture.get('memory'):
                            pair.update(profile=fixture['profile'],driver=fixture['driver'],
                                        source_sha256=fixture['source_sha256'])
                        for side in order:
                            exe=baseline_exe if side=='baseline' else candidate
                            mode=baseline_mode if side=='baseline' else args.candidate_mode
                            pair[side]=_observe(exe,fixture,stage,batch,mode,cpu)
                        pairs.append(pair)
                        journal.append(run_number, seed, pair)
                    print(f'run {run_number+1} {case}/{stage}: {args.pairs} valid pairs',flush=True)
        if (initial_sources!=sources(include_memory=bool(memory_entries))
                or initial_binary!=sha(candidate) or original_binary!=sha(baseline_exe)):
            raise ComparisonError('source/executable changed during measurement')
        if encoder_hash_before is not None and encoder_hash_before!=sha(encoder):
            raise ComparisonError('fixture encoder changed during memory measurement')
        _verify_build_growth_explanation(growth_explanation)
        for f in fixtures:
            if sha(f['path'])!=f['sha256']: raise ComparisonError('fixture changed during measurement')
        baseline.verify_archive(args.baseline); baseline.verify_archive(frozen)
        report['passed']=True; report['source_stable']=True
    except Exception as exc:
        report['error']=str(exc)
        journal.write_report_once(report)
        raise
    finally:
        journal.close()
    journal.write_report_once(report)
    return report

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline',type=Path,required=True)
    parser.add_argument('--candidate',type=Path,required=True)
    parser.add_argument('--selection',required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--pairs',type=int,default=30)
    parser.add_argument('--seed',type=int,default=20261012)
    parser.add_argument('--cpu',type=int)
    parser.add_argument('--candidate-mode',default='decoded')
    parser.add_argument('--build-growth-explanation-file',type=Path)
    parser.add_argument('--workload-subset',nargs='+',help='exact original-mvp three compute IDs; registry selection remains unchanged')
    args=parser.parse_args()
    if args.pairs<30 or args.seed<1: parser.error('at least 30 pairs and positive seed required')
    try: run(args)
    except (OSError,ValueError,subprocess.SubprocessError) as exc:
        print(f'comparison failed: {exc}',file=sys.stderr); return 1
    return 0
if __name__=='__main__': sys.exit(main())
