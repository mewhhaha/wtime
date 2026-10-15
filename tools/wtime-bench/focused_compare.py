"""Predeclared candidate measurements using the existing production timer/oracles.

The exhaustive comparator remains unchanged. Only preparation products alias;
raw pairs always name the one workload actually executed.
"""
from __future__ import annotations
import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys

import baseline
import checkpoint
import cold_products
import compare
import execution_identity
import selection_manifest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SCHEMA = 'wtime-focused-comparison-v1'


def need(ok, message):
    if not ok:
        raise ValueError(message)


def parse_thread_cpus(value, allowed_cpus, *, required=False):
    """Parse an explicit multi-CPU mask and bind it to this process's allowed set."""
    if value is None:
        need(not required, '--thread-cpus is required when thread rows are selected')
        return None
    need(isinstance(value, str) and value.strip(), '--thread-cpus must be a comma-separated CPU list')
    tokens = value.split(',')
    need(all(token.isdigit() for token in tokens), '--thread-cpus contains a non-integer CPU id')
    cpus = [int(token) for token in tokens]
    need(len(cpus) >= 2, '--thread-cpus requires at least two distinct permitted CPUs')
    need(len(cpus) == len(set(cpus)), '--thread-cpus contains duplicate CPU ids')
    need(set(cpus) <= set(allowed_cpus), '--thread-cpus contains a CPU outside available affinity')
    return cpus


def thread_process(executable, args, thread_cpus, timeout=180):
    """Focused-only process runner using the validated multi-CPU affinity mask."""
    result = subprocess.run([str(executable), *map(str, args)], text=True, capture_output=True,
                            timeout=timeout,
                            preexec_fn=lambda: os.sched_setaffinity(0, set(thread_cpus)))
    if result.returncode:
        raise compare.ComparisonError(f'process failed ({result.returncode}): {result.stderr[-4000:]}')
    try:
        data = json.loads(result.stdout)
    except (ValueError, TypeError) as exc:
        raise compare.ComparisonError('missing or malformed process response') from exc
    if not isinstance(data, dict):
        raise compare.ComparisonError('response must be an object')
    return data


def ref(path):
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': compare.sha(path)}


def sources():
    source_map = compare.sources(include_memory=True, include_simd=True,
        include_representative=True, include_integration=True)
    # Thread WAT fixtures are production execution inputs; bind every declared
    # fixture source even though its Wasm is encoded into the run artifact dir.
    for path in [*(HERE/'fixtures'/'threads').glob('*.wat'),
                 *(HERE/'fixtures'/'threads').glob('*.wasm')]:
        source_map[str(path.relative_to(ROOT))] = compare.sha(path)
    return execution_identity.project(source_map, focused=True)


def profile(entry):
    module = entry['oracle'].get('module')
    if module == 'gc_workloads.py':
        return 'all-phase3-core'
    if module == 'memory_workloads.py':
        import memory_scaling
        return memory_scaling._profile(entry)
    if module == 'simd_workloads.py':
        import simd_scaling
        return simd_scaling.execution_profile(entry)
    if module == 'thread_workloads.py':
        return 'threads'
    return entry.get('driver', {}).get('profile', 'mvp')


def input_value(entry):
    """Return the registry row's logical input with value-only SIMD precedence."""
    params = entry.get('params', [])
    if not params:
        return 0
    first = params[0]
    if 'baseline' in first:
        return first['baseline']
    return first.get('value', 0)


def simd_expected_result(entry):
    """Return the normalized exact value or allowed-result set for a SIMD row."""
    import simd_scaling
    import simd_workloads
    values = sorted(int(value.removeprefix('i32:'))
                    for value in simd_scaling.allowed_wire_results(entry, simd_workloads))
    return values[0] if len(values) == 1 else {'allowed_results': values}


def _p128_scalar_row(entry, n, stage, mode, batch):
    """Validate an admitted fixed P128 row and adapt it to the scalar driver API."""
    import scalar_p128_workloads as model
    expected = next((row for row in model.entries() if row['id'] == entry.get('id')), None)
    need(expected is not None, 'unknown P128 scalar workload: '+str(entry.get('id')))
    # The model owns all workload semantics. Registry-only driver metadata is
    # accepted only when it names the existing plain/MVP production driver.
    for key in ('id', 'family', 'source', 'params', 'result', 'oracle',
                'oracle_arguments', 'required_features', 'profile', 'work_units',
                'work_opportunities_per_loop', 'source_event_model',
                'reset_policy', 'stages', 'modes', 'expected_i32',
                'validation_role'):
        need(entry.get(key) == expected.get(key),
             f'P128 registry row differs from admitted model: {entry.get("id")}/{key}')
    need(entry.get('oracle', {}).get('module') == 'scalar_p128_workloads.py',
         'P128 scalar row has the wrong oracle module')
    n_expected = expected['oracle_arguments']['n']
    need(type(n) is int and n == n_expected, 'P128 scalar input differs from admitted fixed input')
    need(stage in model.STAGES, 'unsupported P128 scalar stage: '+str(stage))
    need(mode in model.MODES, 'unsupported P128 scalar mode: '+str(mode))
    need(type(batch) is int and 1 <= batch <= compare.SCALAR_MAX_BATCH, 'invalid P128 scalar batch')
    need(stage == 'warm_call' or batch == 1, 'P128 cold stages require batch one')
    driver = entry.get('driver', {'kind': 'plain', 'profile': model.PROFILE})
    need(driver == {'kind': 'plain', 'profile': model.PROFILE},
         'unsupported P128 scalar driver/profile')
    return {'id': entry['id'], 'driver': driver}, model.oracle(entry['id'], n)


def products(entries, declaration, settings):
    """Declare fresh preparation setup independently of call input/expected result.

    For a shared prepare product the lexically first row supplies every timed
    command (including its untimed preflight). Other input oracles run separately.
    Distinct driver timer implementations and feature profiles cannot alias.
    """
    by_id = {entry['id']: entry for entry in entries}
    result = []
    for selected in declaration['rows']:
        entry = by_id[selected['id']]
        module = entry['oracle'].get('module')
        driver = entry.get('driver', {}).get('kind', module)
        for stage in selected['stages']:
            timer = {'module': module, 'driver': driver,
                     'protocol_sha256': compare.sha(HERE/'src/protocol.rs'),
                     'scalar_sha256': compare.sha(HERE/'src/scalar_driver.rs'),
                     'memory_sha256': compare.sha(HERE/'src/memory_driver.rs')}
            if module == 'gc_workloads.py':
                timer.update({'adapter_sha256': compare.sha(HERE/'gc_cold_adapter.py'),
                              'gc_validator_sha256': compare.sha(HERE/'gc_scaling.py'),
                              'gc_timer_sha256': compare.sha(HERE/'gc_compare.py'),
                              'measurement_policy': 'load_and_call; full typed GC/exception state oracle'})
            if module == 'root_workloads.py':
                timer.update({'driver_sha256': compare.sha(HERE/'src/gc_roots_driver.rs'),
                              'validator_sha256': compare.sha(HERE/'root_compare.py'),
                              'measurement_policy': 'sum_guest_invoke_intervals_excluding_each_oracle_and_gc_reset'})
            if module == 'thread_workloads.py':
                timer.update({'thread_adapter_sha256': compare.sha(HERE/'thread_compare.py'),
                              'thread_scaling_sha256': compare.sha(HERE/'thread_scaling.py'),
                              'thread_model_sha256': compare.sha(HERE/'thread_workloads.py'),
                              'thread_driver_sha256': compare.sha(HERE/'src/thread_driver.rs'),
                              'thread_fixture_wat_sha256': entry['source']['sha256'],
                              'thread_fixture_wasm_sha256': entry['source']['wasm_sha256'],
                              'measurement_policy': 'complete steady_state_group driver batches; retain all legacy trials'})
            result.append({'workload': entry['id'], 'stage': stage,
                'wasm_sha256': entry['source']['wasm_sha256'], 'profile': profile(entry),
                'runtime_settings': settings,
                'timer': timer,
                'setup': {'policy': 'fresh process; canonical product row preflight; fresh Engine compile',
                          'canonical_row': 'lexicographically first equivalent declared workload'}})
    return result


def prepare_fixtures(entries, frozen, output, encoder):
    fixtures = {}
    import subprocess
    for entry in entries:
        source = entry['source']
        if entry['oracle'].get('module') == 'gc_workloads.py':
            import gc_workloads
            wat = HERE/source['path']
            need(wat.is_file() and compare.sha(wat) == source['sha256'], 'WAT source changed: '+entry['id'])
            wasm = output/(entry['id']+'.wasm')
            proc = subprocess.run([str(encoder), 'encode', str(wat), '-o', str(wasm)],
                                  cwd=ROOT, text=True, capture_output=True, timeout=60)
            need(proc.returncode == 0, 'GC fixture encoding failed: '+proc.stderr[-2000:])
            need(compare.sha(wasm) == source['wasm_sha256'], 'GC Wasm bytes changed: '+entry['id'])
            setup_paths = []
            for index, setup in enumerate(entry.get('setup_modules', [])):
                setup_wat = HERE/setup['path']
                need(setup_wat.is_file() and compare.sha(setup_wat) == setup['sha256'],
                     'GC setup WAT source changed: '+entry['id'])
                setup_wasm = output/(entry['id']+'-setup-'+str(index)+'.wasm')
                proc = subprocess.run([str(encoder), 'encode', str(setup_wat), '-o', str(setup_wasm)],
                                      cwd=ROOT, text=True, capture_output=True, timeout=60)
                need(proc.returncode == 0, 'GC setup fixture encoding failed: '+proc.stderr[-2000:])
                need(compare.sha(setup_wasm) == setup['wasm_sha256'],
                     'GC setup Wasm bytes changed: '+entry['id'])
                setup_paths.append(setup_wasm)
            fixtures[entry['id']] = {'path': wasm, 'setup_modules': setup_paths}
            continue
        if entry['oracle'].get('module') == 'thread_workloads.py':
            wat = HERE/source['path']
            need(wat.is_file() and compare.sha(wat) == source['sha256'],
                 'thread WAT fixture source changed: '+entry['id'])
            wasm = output/(entry['id']+'.wasm')
            proc = subprocess.run([str(encoder), 'encode', str(wat), '-o', str(wasm)],
                                  cwd=ROOT, text=True, capture_output=True, timeout=60)
            need(proc.returncode == 0, 'thread fixture encoding failed: '+proc.stderr[-2000:])
            need(compare.sha(wasm) == source['wasm_sha256'], 'thread Wasm fixture bytes changed: '+entry['id'])
            fixtures[entry['id']] = wasm
            continue
        wat = HERE/source['path']
        if entry['id'] == 'prepare_large':
            import workloads
            wat = output/'prepare_large.wat'
            wat.write_text(workloads.generate_prepare_large()+'\n')
        need(wat.is_file() and compare.sha(wat) == source['sha256'], 'WAT source changed: '+entry['id'])
        wasm = wat.with_suffix('.wasm')
        if not wasm.is_file() or compare.sha(wasm) != source['wasm_sha256']:
            wasm = frozen/'fixtures'/(entry['id']+'.wasm')
        if not wasm.is_file() or compare.sha(wasm) != source['wasm_sha256']:
            wasm = compare._encode_scalar_fixture(entry, output, encoder)
        need(compare.sha(wasm) == source['wasm_sha256'], 'Wasm bytes changed: '+entry['id'])
        fixtures[entry['id']] = wasm
    return fixtures


def observe(executable, entry, wasm, stage, batch, mode, cpu, output, seed=None,
            thread_cpus=None, fuel_setting=None, register_entry_threshold=None):
    module = entry['oracle'].get('module')
    n = input_value(entry)
    entry_options = ({'register_entry_threshold': register_entry_threshold}
                     if register_entry_threshold is not None else {})
    if fuel_setting is not None:
        import p127_fuel_policy
        p127_fuel_policy.cli_value(fuel_setting)
        need(module in ('scalar_workloads.py','run.py') or entry['id']=='prepare_large',
             'explicit source fuel is supported only by scalar or single-case drivers')
    if module == 'thread_workloads.py':
        import thread_compare
        need(stage == thread_compare.STAGE, 'thread rows require their declared steady_state_group stage')
        need(mode == thread_compare.MODE, 'focused thread timer requires decoded execution mode')
        need(seed is not None, 'thread observation requires a stable run/pair seed')
        need(thread_cpus is not None, 'thread observations require the explicit thread CPU mask')
        return thread_compare.observe(executable, wasm, entry, seed=seed, process=thread_process,
                                      cpu=thread_cpus, thread_cpu_affinity=thread_cpus, batch=batch)
    if module == 'root_workloads.py':
        import root_compare
        wire = compare.process(executable, ['--gc-root-driver', wasm, entry['id'], entry['family'],
            n, mode, profile(entry), stage, batch], cpu)
        return root_compare.validate_observation(entry, wire, batch, expected_mode=mode, expected_stage=stage)
    if module == 'gc_workloads.py':
        import gc_cold_adapter
        import gc_compare
        gc_cold_adapter.process = compare.process
        gc_cold_adapter.ComparisonError = compare.ComparisonError
        sample = gc_cold_adapter.cold_observation(executable, wasm, entry, n, stage, batch, mode, cpu)
        return gc_compare._normalize_cold(entry, sample, __import__('gc_workloads'))
    if module == 'scalar_workloads.py':
        import scalar_workloads as model
        return compare.scalar_observation(executable, wasm, entry, n, model.oracle(entry['id'], n),
            stage, batch, mode, cpu, **entry_options,
            **({'fuel_setting':fuel_setting} if fuel_setting is not None else {}))
    if module == 'scalar_p128_workloads.py':
        need(fuel_setting is None, 'explicit source fuel is unsupported for P128 scalar rows')
        driver_entry, expected = _p128_scalar_row(entry, n, stage, mode, batch)
        return compare.scalar_observation(executable, wasm, driver_entry, n, expected,
            stage, batch, mode, cpu, **entry_options)
    if module == 'memory_workloads.py':
        import memory_workloads as model
        return compare.memory_observation(executable, wasm, entry, n, model.expected_memory(entry['id'], n),
            stage, batch, mode, cpu, memory_module=model)
    if module == 'simd_workloads.py':
        import simd_workloads as model
        import simd_scaling as scaling
        raw = next(entry['oracle_arguments'][key] for key in ('n','case','index','vectors','value')
                   if key in entry['oracle_arguments'])
        return compare._simd_observation(executable, wasm, entry, scaling.typed_input(entry), raw,
            scaling.allowed_wire_results(entry, model), stage, batch, mode, profile(entry), cpu,
            model=model, expected_batch=batch)
    if module == 'representative_workloads.py':
        import representative_workloads as model
        import representative_scaling as scaling
        import representative_compare as timer
        expected = model.oracle(entry['id'], n)
        wire = compare.process(executable, ['--representative-staged', wasm, entry['id'], n,
            'i32:'+str(expected['result']), expected['output_sha256'], stage, batch, mode, profile(entry)], cpu)
        return timer.retain_memory_image(timer.validate_observation(entry, wire, model, scaling,
            batch, expected_mode=mode), output)
    need(module == 'run.py' or entry['id'] == 'prepare_large', 'unsupported candidate oracle: '+str(module))
    import run as model
    expected = model.oracle(entry['id'], n) if entry['id'] != 'prepare_large' else 0
    return compare.observation(executable, wasm, entry['id'], n, expected, stage, batch, mode, cpu,
        **entry_options, **({'fuel_setting':fuel_setting} if fuel_setting is not None else {}))


def batch_limit(entry):
    """Driver-specific capacity, including each resource-sensitive work bound."""
    maximum = 100_000
    if entry['oracle'].get('module') in ('run.py', 'scalar_workloads.py', 'scalar_p128_workloads.py'):
        maximum = compare.SCALAR_MAX_BATCH
    if entry['oracle'].get('module') == 'thread_workloads.py':
        maximum = 128
    if entry['oracle'].get('module') == 'representative_workloads.py':
        import representative_compare
        maximum = representative_compare.cap(entry)
    if entry['oracle'].get('module') == 'root_workloads.py':
        import root_workloads
        maximum = root_workloads.max_batch(entry['gc_expectation']['bank_slots'], input_value(entry))
    if entry['oracle'].get('module') == 'simd_workloads.py':
        import simd_scaling
        maximum = simd_scaling.batch_limit(entry)
    return maximum


def calibrate(entry, observer):
    """Confirm the same requested batch reaches 20 ms on BOTH actual sides."""
    workload_id = entry.get('id', '<unknown>')
    maximum = batch_limit(entry)
    batch = 1
    audit = []
    for _ in range(8):
        rows = {side: observer(side, batch) for side in ('baseline', 'candidate')}
        need(all(row['batch'] == batch for row in rows.values()), 'calibration batch mismatch')
        times = [row['ns_per_operation'] * batch for row in rows.values()]
        need(all(math.isfinite(value) and value > 0 for value in times), 'invalid calibration timing')
        audit.append({'batch': batch, 'observations': rows})
        if min(times) >= 20_000_000:
            return batch, audit
        if batch >= maximum:
            totals = ', '.join(f'{side}={rows[side]["ns_per_operation"] * batch:.0f}ns '
                               f'({rows[side]["ns_per_operation"]:.6g}ns/op)'
                               for side in ('baseline', 'candidate'))
            raise ValueError(f'{workload_id}: calibration reached batch cap {maximum}; '
                             f'final batch {batch}, side totals below 20000000ns: {totals}; '
                             'calibration cannot reach 20 ms')
        next_batch = min(maximum, max(batch + 1, math.ceil(batch * 20_000_000/min(times) * 1.1)))
        need(next_batch > batch,
             f'{workload_id}: calibration cannot reach 20 ms; final batch {batch}, '
             f'baseline={times[0]:.0f}ns, candidate={times[1]:.0f}ns, cap={maximum}')
        batch = next_batch
    raise ValueError('calibration failed to reach 20 ms on both sides')


def run(args):
    need(args.smoke or args.frozen is not None, 'candidate timings require a frozen selection')
    need(args.smoke or getattr(args,'semantic_receipt',None) is not None,
         'candidate timings require the successful semantic verification receipt')
    declaration = (selection_manifest.verify_frozen(args.manifest, args.frozen) if args.frozen
                   else selection_manifest.load(args.manifest))
    entries = selection_manifest.rows(declaration)
    import p140_entry_policy
    entry_setting = p140_entry_policy.request(declaration, args.candidate_mode,
                                             getattr(args, 'register_entry_threshold', None))
    if entry_setting is not None:
        need(all(entry['oracle'].get('module') in ('scalar_workloads.py', 'scalar_p128_workloads.py', 'run.py')
                 or entry['id'] == 'prepare_large' for entry in entries),
             'adaptive entry requires scalar or single-case rows')
    fuel_token = getattr(args, 'source_fuel', None)
    fuel_setting = None
    if fuel_token is not None:
        import p127_fuel_policy
        fuel_setting = p127_fuel_policy.parse_cli(fuel_token)
        need(all(entry['oracle'].get('module') in ('scalar_workloads.py','run.py')
                 or entry['id']=='prepare_large' for entry in entries),
             'explicit source fuel requires a scalar-only declaration')
    allowed = sorted(os.sched_getaffinity(0))
    cpu = args.cpu if args.cpu is not None else allowed[0]
    need(cpu in allowed, 'CPU outside permitted affinity')
    has_threads = any(entry['oracle'].get('module') == 'thread_workloads.py' for entry in entries)
    thread_cpus = parse_thread_cpus(args.thread_cpus, allowed, required=has_threads)
    need(has_threads or thread_cpus is None, '--thread-cpus requires selected thread rows')
    frozen_selection = json.loads(args.frozen.read_text()) if args.frozen else None
    archive = baseline.verify_archive(args.baseline)
    # The supplied adapter may wrap the declared predecessor, but never substitute it.
    predecessor = args.baseline if archive.get('kind') != 'frozen-typed-adapter' else Path(archive['baseline'])
    if frozen_selection:
        need(compare.sha(predecessor/'manifest.json') == frozen_selection['predecessor_sha256']
             and (predecessor/'manifest.json').resolve() == Path(frozen_selection['predecessor_path']),
             'benchmark baseline is not the frozen selection predecessor')
    info = compare._baseline_identity(args.baseline, archive, HERE/'src/protocol.rs', baseline)
    candidate = args.candidate.resolve(strict=True)
    baseline_binary = Path(info['executable'])
    before = sources()
    identities = {'candidate': ref(candidate), 'baseline': ref(baseline_binary),
                  'baseline_manifest': ref(args.baseline/'manifest.json'),
                  'selection_manifest': ref(args.manifest)}
    if args.frozen:
        identities['selection_freeze'] = ref(args.frozen)
    if getattr(args,'semantic_receipt',None):
        identities['semantic_receipt'] = ref(args.semantic_receipt)
        import semantic_receipt
        semantic_receipt.validate(args.semantic_receipt, declaration, before, ROOT)
    capabilities = {side: compare.process(exe, ['--protocol'], cpu)
                    for side, exe in (('baseline', baseline_binary), ('candidate', candidate))}
    settings = {'instrumented': False, 'profile': 'release', 'fuel_policy': False, 'cancellation_policy': 1,
                'baseline_mode': info['mode'], 'candidate_mode': args.candidate_mode,
                'thread_cpu_affinity': thread_cpus,
                'workload_profiles':{entry['id']:profile(entry) for entry in entries}}
    if fuel_setting is not None:
        settings['fuel_policy'] = fuel_setting
    if entry_setting is not None:
        settings['register_entry_threshold'] = entry_setting
        p140_entry_policy.capability(capabilities['candidate'].get('register_entry_threshold_contract'))
    for side, cap in capabilities.items():
        need(cap.get('schema_version') == 2 and cap.get('instrumented') is False
             and settings[side+'_mode'] in cap.get('modes', []), 'unsupported production mode: '+side)
        if fuel_setting is not None:
            need(cap.get('source_fuel_contract') == p127_fuel_policy.protocol_contract(),
                 'both paired sides require the same explicit source-fuel contract')
        required_profiles = {profile(entry) for entry in entries}
        if any(entry['oracle'].get('module') == 'thread_workloads.py' for entry in entries):
            required_profiles.discard('threads')
        need(required_profiles <= set(cap.get('features', [])), 'unsupported feature profile')
        if any(entry['oracle'].get('module') in ('gc_workloads.py', 'root_workloads.py') for entry in entries):
            need(settings[side+'_mode'] == 'decoded',
                 'legacy GC candidate rows require the existing decoded cold driver on both sides')
        if any(entry['oracle'].get('module') == 'thread_workloads.py' for entry in entries):
            import thread_compare
            thread_compare.validate_capability(cap)
    args.output.mkdir(parents=True, exist_ok=False)
    # Source copies precede execution; the outer shared runner binds the command.
    for name, digest in before.items():
        path = ROOT/name
        dest = args.output/'sources'/name
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, dest)
        need(compare.sha(dest) == digest, 'source changed while archiving')
    command_path = args.output/'command.json'
    command_path.write_text(json.dumps({'argv': sys.argv, 'cwd':str(ROOT),
        'thread_cpu_affinity': thread_cpus}, indent=2)+'\n')
    identities['command'] = ref(command_path)
    fixture_dir = args.output/'fixtures'
    fixture_dir.mkdir()
    encoder = ROOT/'target/debug/wtime-conformance'
    encoder_identity = ref(encoder)
    fixture_paths = prepare_fixtures(entries, predecessor, fixture_dir, encoder)
    identities['fixture_files'] = {wid:ref(path['path'] if isinstance(path, dict) else path)
                                   for wid,path in fixture_paths.items()}
    identities['setup_fixture_files'] = {
        wid: [ref(path) for path in fixture['setup_modules']]
        for wid,fixture in fixture_paths.items() if isinstance(fixture, dict) and fixture['setup_modules']}
    product_plan = cold_products.plan(products(entries, declaration, settings))
    by_id = {entry['id']: entry for entry in entries}
    def seed_for(*parts):
        digest = hashlib.sha256(json.dumps([args.seed, *parts], separators=(',', ':')).encode()).digest()
        return (int.from_bytes(digest[:8], 'big') & ((1 << 63) - 1)) or 1
    def call(side, wid, stage, batch, seed=None):
        exe = baseline_binary if side == 'baseline' else candidate
        if by_id[wid]['oracle'].get('module') == 'thread_workloads.py' and seed is None:
            seed = seed_for('observation', wid, stage, batch)
        return observe(exe, by_id[wid], fixture_paths[wid], stage, batch, settings[side+'_mode'], cpu,
                       args.output, seed=seed, thread_cpus=thread_cpus,
                       **({'register_entry_threshold': entry_setting[side]} if entry_setting is not None else {}),
                       **({'fuel_setting':fuel_setting} if fuel_setting is not None else {}))
    report = {'schema': SCHEMA, 'schema_version':2, 'selection':declaration['task'],
        'lane':'development' if args.smoke else 'candidate', 'passed':False, 'source_stable':False,
        'declaration': declaration, 'identities':identities, 'cold_products':product_plan,
        'recorded_at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'metadata':{'settings':settings, 'capabilities':capabilities, 'candidate_sources':before,
            'candidate_binary_sha256':identities['candidate']['sha256'],
            'baseline_binary_sha256':identities['baseline']['sha256'],
            'protocol_sha256':compare.sha(HERE/'src/protocol.rs'), 'fixture_encoder':encoder_identity,
            'hardware':{'selected_cpu':cpu, 'thread_cpu_affinity':thread_cpus,
                'cpu_affinity_available':allowed,
                'cpu_model':next((line.split(':',1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines()
                    if line.startswith('model name')), 'unknown'),
                'cpu_quota':Path('/sys/fs/cgroup/cpu.max').read_text().strip(), 'physical_core_idle':'unknown/shared cloud'}},
        'correctness_probes':[], 'warm_calibrations':[], 'runs':[]}
    run_ids = (0,) if args.smoke else (0,1)
    pair_count = 1 if args.smoke else 30
    seeds = {run:args.seed+run*1_000_003 for run in run_ids}
    journal = checkpoint.Journal(args.output, {'identities':identities, 'sources':before, 'settings':settings,
        'seed':args.seed, 'pairs':pair_count}, seeds, prepared=True)
    try:
        # All selected inputs receive full independent oracle checks, including alias members.
        for entry in entries:
            stage = 'steady_state_group' if entry['oracle'].get('module') == 'thread_workloads.py' else (
                'prepare' if entry['id'] == 'prepare_large' else (
                'run_after_setup' if entry['oracle'].get('module') == 'representative_workloads.py' else 'load_and_call')
            )
            for side in ('baseline', 'candidate'):
                report['correctness_probes'].append({'workload':entry['id'], 'stage':stage, 'side':side,
                    'observation':call(side, entry['id'], stage, 1,
                        seed=seed_for('correctness_probe', entry['id'], stage)), 'timing_excluded':True})
        scalar = [entry for entry in entries if entry['oracle'].get('module') == 'scalar_workloads.py']
        if scalar:
            import scalar_workloads
            for side, exe in (('baseline', baseline_binary), ('candidate', candidate)):
                compare._preflight_scalar_side(exe, settings[side+'_mode'], scalar, fixture_paths, cpu, scalar_workloads)
        for run_id in run_ids:
            rows = []
            report['runs'].append({'run_id':run_id, 'seed':seeds[run_id], 'pairs':rows})
            for group in product_plan['groups']:
                wid, stage = group['sample_workload'], group['stage']
                entry = by_id[wid]
                batch = 1
                thread_stage = entry['oracle'].get('module') == 'thread_workloads.py' and stage == 'steady_state_group'
                if not args.smoke and (stage in ('warm_call', 'run_after_setup') or thread_stage):
                    cal_seed = seed_for('calibration', run_id, wid, stage)
                    batch, audit = calibrate(entry, lambda side, n:call(side, wid, stage, n, seed=cal_seed))
                    report['warm_calibrations'].append({'run_id':run_id, 'workload':wid, 'stage':stage,
                        'batch':batch, 'target_ns':20_000_000, 'attempts':audit})
                orders = compare.pair_order(seeds[run_id]+int(group['product_sha256'][:8],16), pair_count)
                for sample, order in enumerate(orders):
                    pair_seed = seed_for('pair', run_id, wid, stage, sample)
                    pair = {'pair_id':f'{run_id}:{wid}:{stage}:{sample}', 'workload':wid, 'stage':stage,
                        'input':input_value(entry),
                        'wasm_sha256':entry['source']['wasm_sha256'], 'order':order,
                        'product_sha256':group['product_sha256']}
                    for side in order:
                        pair[side] = call(side, wid, stage, batch, seed=pair_seed)
                    if entry['oracle'].get('module') == 'simd_workloads.py':
                        pair['expected'] = simd_expected_result(entry)
                    else:
                        pair['expected'] = pair['baseline']['result']
                    need(pair['baseline']['result'] == pair['expected']
                         and pair['candidate']['result'] == pair['expected'], 'paired oracle mismatch')
                    rows.append(pair)
                    journal.append(run_id,seeds[run_id],pair)
        need(sources() == before, 'measured source changed')
        for identity in [*(value for key,value in identities.items()
                            if key not in ('fixture_files', 'setup_fixture_files')),
                         *identities['fixture_files'].values(),
                         *(ref for values in identities['setup_fixture_files'].values() for ref in values),
                         encoder_identity]:
            need(compare.sha(identity['path']) == identity['sha256'], 'measured artifact changed')
        report['counts'] = cold_products.summarize(product_plan, {
            group['sample_workload']+'\0'+group['stage']:len(run_ids)*pair_count for group in product_plan['groups']})
        report['passed'] = report['source_stable'] = True
    except BaseException as exc:
        report['error'] = type(exc).__name__+': '+str(exc)
        raise
    finally:
        journal.close()
        journal.write_report_once(report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('manifest','baseline','candidate','output'):
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--frozen',type=Path)
    parser.add_argument('--semantic-receipt',type=Path)
    parser.add_argument('--candidate-mode', choices=('decoded','compact','register'), required=True)
    parser.add_argument('--register-entry-threshold', type=int,
                        help='explicit candidate Register threshold, bound to the frozen selection')
    parser.add_argument('--source-fuel', choices=('none','50000000'),
                        help='explicit scalar policy, applied identically to both paired sides')
    parser.add_argument('--thread-cpus', help='explicit comma-separated affinity mask for thread rows (at least two permitted CPUs)')
    parser.add_argument('--seed', type=int, default=20261013)
    parser.add_argument('--cpu', type=int)
    parser.add_argument('--smoke', action='store_true', help='one pair, one run; never promotion evidence')
    args = parser.parse_args()
    report = run(args)
    print(json.dumps({'passed':report['passed'], 'lane':report['lane'], 'counts':report['counts']}))


if __name__ == '__main__':
    main()
