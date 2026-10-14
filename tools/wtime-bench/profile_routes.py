"""Staged bounded P035 diagnostics: real13 supported cases; explicit116 gaps.

All129 selected IDs are recorded, but production paired coverage is a different
artifact. Existing generic diagnostics cover only3 compute controls+memory_fill;
new representative companion validates9 full-memory oracles. No fake counters.
"""
from __future__ import annotations
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[1]
COUNTER_LEGACY={'call_overhead','integer_loop','branch_calls','memory_fill_scan'}
COUNTS={'scalar':85,'memory':35,'representative':9}


def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def prepare_fixture(entry, tool_dir, output_root, encoder, *, runner=subprocess.run):
    source=entry['source'];wat=Path(tool_dir)/source['path']
    if not wat.is_file() or sha(wat)!=source['sha256']:raise ValueError('actual WAT source hash mismatch')
    output_root=Path(output_root);output_root.mkdir(parents=True,exist_ok=True)
    wasm=output_root/(source['sha256']+'.wasm')
    command=[str(encoder),'encode',str(wat),'-o',str(wasm)]
    cached=wasm.exists()
    if not cached:
        result=runner(command,text=True,capture_output=True,timeout=60)
        if result.returncode:raise ValueError('actual encoding failed: '+result.stderr[-2000:])
    if not wasm.is_file() or sha(wasm)!=source['wasm_sha256']:raise ValueError('actual encoded Wasm hash mismatch')
    return wat,wasm,{'command':command,'exit_code':None if cached else 0,'executed':not cached,'cached_same_source':cached,'encoder_sha256':sha(encoder),'wat_sha256':sha(wat),'wasm_sha256':sha(wasm)}


def independent_rows(selection,selected):
    import scalar_workloads
    import memory_workloads
    import representative_workloads
    if selection=='scalar':
        ids=['call_overhead','integer_loop','branch_calls']
        if not ([row['id'] for row in selected[:3]]==ids):raise ValueError('independent selection differs')
        if not (selected[3:]==scalar_workloads.entries()):raise ValueError('independent selection differs')
    elif selection=='memory':
        if not (selected[0]['id']=='memory_fill_scan'):raise ValueError('independent selection differs')
        if not (selected[1:]==memory_workloads.entries()):raise ValueError('independent selection differs')
    else:
        if not (selected==representative_workloads.entries()):raise ValueError('independent representative selection differs')


def counter_route(entry,selection_name):
    """Only explicitly declared i32/MVP legacy or full representative routes."""
    oracle=entry.get('oracle',{})
    if selection_name=='representative':
        if oracle.get('module')!='representative_workloads.py':
            raise ValueError('representative profile requires independent representative oracle')
        return 'representative'
    if entry.get('id') in COUNTER_LEGACY:
        params=entry.get('params',[])
        if (oracle.get('module')!='run.py' or oracle.get('function')!='oracle'
                or oracle.get('independent') is not True or entry.get('required_features')!=['mvp']
                or len(params)!=1 or params[0].get('type')!='i32' or entry.get('result',{}).get('type')!='i32'):
            raise ValueError('legacy profile requires exact independent i32/MVP oracle contract')
        return 'legacy'
    if oracle.get('module') not in {'scalar_workloads.py','memory_workloads.py'}:
        raise ValueError('unknown counter/profile oracle route')
    return 'unsupported'


def run_profile(selection_name,output,*,cpu=None,runner=subprocess.run,which=None):
    import shutil
    import profile as tools
    import compare
    import workloads
    import run as original
    import representative_workloads as model
    import representative_scaling as scaling
    import representative_compare
    which=shutil.which if which is None else which
    selected=workloads.selection(selection_name)
    if selection_name not in COUNTS or len(selected)!=COUNTS[selection_name]:raise tools.ProfileError('wrong full diagnostic selection')
    independent_rows(selection_name,selected)
    out=Path(output).resolve()
    if out.exists():raise tools.ProfileError('profile output must be exclusively fresh')
    out.mkdir(parents=True,exist_ok=False)
    cpus=sorted(os.sched_getaffinity(0));cpu=cpus[0] if cpu is None else cpu
    if cpu not in cpus:raise tools.ProfileError('CPU outside actual affinity')
    def source_inputs():
        combined=compare.sources(include_memory=True,include_representative=True)
        combined.update(tools.source_inputs())
        return combined
    before=source_inputs()
    report={'schema_version':1,'passed':False,'selection':selection_name,
            'recorded_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'evidence_kind':'bounded_diagnostic_attribution','production_timing':False,
            'metadata':{'source_inputs':before,'host':{'selected_cpu':cpu,'cpu_affinity_available':cpus},
                        'counter_policy':{'feature':'performance-counters','instrumented':True,
                                          'scope':'3 legacy compute +1 originalmemory +9 setup-aware representative'},
                        'sampling_policy':{'separate_binary':True,'diagnostic_debug_flags':True},
                        'artifacts':{},'tools':{}},
            'cases':[],'counters':[],'cpu_profiles':[],
            'limitations':['This report records all selected IDs but profiles only explicitly supported counter routes.',
                           'Counter counts are attribution clues, never production latency or CPU percentages.',
                           'Hardware sampling availability and symbolization are recorded independently.']}
    report_path=out/'profile.json'
    def save():report_path.write_text(json.dumps(report,indent=2,sort_keys=True,allow_nan=False)+'\n')
    save()
    try:
        report['metadata']['tools']=tools.probe_tools(out,which=which,runner=runner)
        sample_command,sample_flags,sample_binary=tools._build(out/'sampling-target',diagnostic=False,runner=runner)
        counter_command,counter_flags,counter_binary=tools._build(out/'counter-target',diagnostic=True,runner=runner)
        if sample_binary.resolve()==counter_binary.resolve():raise tools.ProfileError('counter/sampling paths must be distinct')
        report['metadata']['artifacts']={name:{'binary':str(binary.relative_to(out)),'binary_sha256':sha(binary),
                                              'build_command':command,'build_flags':flags}
                                       for name,binary,command,flags in [('sampling',sample_binary,sample_command,sample_flags),
                                                                       ('counters',counter_binary,counter_command,counter_flags)]}
        (out/'counters').mkdir();(out/'sampling').mkdir()
        encoder=compare.ROOT/'target/debug/wtime-conformance'
        encoder_before=sha(encoder)
        report['metadata']['fixture_encoder_sha256']=encoder_before
        for entry in selected:
            case=entry['id'];source=entry['source']
            wat,wasm,encoding=prepare_fixture(entry,HERE,out/'fixtures',encoder,runner=runner)
            record={'workload':case,'family':entry.get('family', 'legacy_' + case),'fixture_sha256':sha(wasm),'wat_sha256':sha(wat),'encoding':encoding}
            route=counter_route(entry,selection_name)
            supported=route!='unsupported'
            if not supported:
                reason='No declared counter/profile route for this modern family in the bounded P035 diagnostic scope; execution support is not in question.'
                report['cases'].append({**record,'counter_status':'unsupported','reason':reason})
                report['cpu_profiles'].append({'workload':case,'available':False,'capability_gap':reason,'attempted':False})
                continue
            n=entry['params'][0]['baseline'];raw=None
            if selection_name=='representative':
                expected=model.oracle(case,n)
                counter_args=['--representative-staged-diagnostic',str(wasm),case,str(n),f"i32:{expected['result']}",
                              expected['output_sha256'],'run_after_setup','1','decoded','mvp']
                raw=tools._json_command([str(counter_binary),*counter_args],runner,cpu=cpu)
                representative_compare.validate_observation(entry,raw,model,scaling,1,diagnostic=True)
                if raw.get('diagnostic') is not True:raise tools.ProfileError(case+': missing diagnostic flag')
                normalized={'schema_version':1,'diagnostic':True,'result':expected['result'],'counters':raw.get('counters')}
                counters=tools.validate_counter_response(normalized,expected['result'])
                sample_batch=min(tools.PROFILE_BATCH,representative_compare.cap(entry))
                sample_args=['--representative-staged',str(wasm),case,str(n),f"i32:{expected['result']}",expected['output_sha256'],
                             'run_after_setup',str(sample_batch),'decoded','mvp']
                oracle_value=expected['result']
            else:
                expected=original.oracle(case,n)
                counter_args=['--diagnostic',str(wasm),str(n),str(expected),'decoded','mvp']
                raw=tools._json_command([str(counter_binary),*counter_args],runner,cpu=cpu)
                counters=tools.validate_counter_response(raw,expected)
                sample_batch=min(tools.PROFILE_BATCH,max(1,50_000_000//max(n,1)))
                sample_args=['--single-case',str(wasm),case,str(n),str(expected),'warm_call',str(sample_batch),'decoded']
                oracle_value=expected
            # Validate the separate sampling executable before any perf request.
            preflight_args=sample_args.copy()
            preflight_args[7 if selection_name=='representative' else 6]='1'
            sample_raw=tools._json_command([str(sample_binary),*preflight_args],runner,cpu=cpu)
            def validate_sample(raw,batch):
                if selection_name=='representative':
                    return representative_compare.validate_observation(entry,raw,model,scaling,batch)
                if raw.get('schema_version')!=2 or raw.get('case')!=case or raw.get('result')!=oracle_value or raw.get('expected')!=oracle_value or raw.get('batch')!=batch:
                    raise tools.ProfileError(case+': sampling guest result/identity/batch mismatch')
                elapsed=raw.get('ns_per_operation')
                if type(elapsed) not in (int,float) or not math.isfinite(elapsed) or elapsed<=0:
                    raise tools.ProfileError(case+': sampling guest time invalid')
                return raw
            validate_sample(sample_raw,1)
            counter_row={**record,'input':n,'expected':oracle_value,'binary_sha256':sha(counter_binary),
                         'command':[str(counter_binary),*counter_args],'counters':counters,
                         'raw_observation':raw,'sampling_preflight':sample_raw,'instrumented':True,'role':'diagnostic_profile'}
            path=out/'counters'/(case+'.json');path.write_text(json.dumps(counter_row,indent=2,sort_keys=True,allow_nan=False)+'\n')
            counter_row['artifact']=str(path.relative_to(out));counter_row['artifact_sha256']=sha(path)
            report['counters'].append(counter_row);report['cases'].append({**record,'counter_status':'passed','counter_artifact':str(path.relative_to(out))})
            sampling=report['metadata']['tools'].get('sampling',{})
            perf=report['metadata']['tools'].get('perf',{}).get('path')
            cpu_row={'workload':case,'available':False,'attempted':False}
            if not sampling.get('available') or not perf:
                cpu_row['capability_gap']=sampling.get('reason','perf sampling not available')
            else:
                data=out/'sampling'/(case+'.perf.data');text=out/'sampling'/(case+'.perf-report.txt')
                command=[perf,'record','--call-graph','dwarf','-F','99','-o',str(data),'--',str(sample_binary),*sample_args]
                cpu_row.update(command=command,attempted=True)
                done=runner(command,cwd=ROOT,text=True,capture_output=True,timeout=600,preexec_fn=tools._affinity(cpu))
                cpu_row.update(exit_code=done.returncode,stdout=done.stdout,stderr=done.stderr)
                if done.returncode!=0:
                    raise tools.ProfileError(case+': perf was probed available but record/guest process failed: '+done.stderr[-1200:])
                else:
                    try: perf_wire=json.loads(done.stdout)
                    except json.JSONDecodeError as error: raise tools.ProfileError(case+': perf guest returned malformed JSON') from error
                    validate_sample(perf_wire,sample_batch)
                    decoded=runner([perf,'report','--stdio','--input',str(data),'--sort','symbol,dso'],cwd=ROOT,text=True,capture_output=True,timeout=60)
                    cpu_row['report_exit_code']=decoded.returncode
                    if decoded.returncode!=0:cpu_row['capability_gap']='actual perf report symbolization failed'
                    else:
                        text.write_text(decoded.stdout);cpu_row.update(available=True,data=str(data.relative_to(out)),data_sha256=sha(data),report=str(text.relative_to(out)),report_sha256=sha(text))
            report['cpu_profiles'].append(cpu_row);save()
        expected_supported={'scalar':3,'memory':1,'representative':9}[selection_name]
        if len(report['cases'])!=len(selected) or len(report['counters'])!=expected_supported:raise tools.ProfileError('bounded selected/profiled matrix incomplete')
        if source_inputs()!=before:raise tools.ProfileError('sources changed during actual diagnostics')
        for name,binary in [('sampling',sample_binary),('counters',counter_binary)]:
            if sha(binary)!=report['metadata']['artifacts'][name]['binary_sha256']:raise tools.ProfileError('diagnostic binary identity changed')
        if sha(encoder)!=encoder_before:raise tools.ProfileError('fixture encoder changed')
        report['counts']={'selected':len(selected),'actual_counter_cases':len(report['counters']),
                          'explicit_unsupported_cases':len(selected)-len(report['counters']),
                          'actual_available_cpu_profiles':sum(bool(r['available']) for r in report['cpu_profiles'])}
        report['passed']=True
    except Exception as error:
        report['error']=type(error).__name__+': '+str(error);save();raise
    save();return report
