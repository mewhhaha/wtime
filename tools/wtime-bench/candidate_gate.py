"""Validate a frozen candidate selection and decide it without timing Wasm."""
from __future__ import annotations
import argparse
import hashlib
import json
import gzip
import math
from pathlib import Path
import random
import sys

import analyze
import baseline
import cold_products
import compare
import execution_identity
import focused_compare
import selection_manifest
from bootstrap import draw_median


def need(ok, message):
    if not ok:
        raise ValueError(message)


def expected(entry):
    import importlib
    module = entry['oracle']['module'].removesuffix('.py')
    model = importlib.import_module(module)
    if entry['id'] == 'prepare_large':
        return None
    if module == 'thread_workloads':
        import thread_compare
        return thread_compare.expected(entry)
    n = focused_compare.input_value(entry)
    if module == 'simd_workloads':
        return focused_compare.simd_expected_result(entry)
    value = model.oracle(entry['id'], n)
    return value['result'] if module == 'representative_workloads' else value


def check_observation(entry, observation, mode, root, stage=None, thread_cpus=None, fuel_setting=None):
    module = entry['oracle']['module']
    if module == 'continuation_live_workloads.py':
        import continuation_live_workloads as model
        need(mode == model.MODE and stage in model.STAGES,
             'live-continuation observation mode/stage differs')
        wire = observation.get('raw_wire_observation')
        need(isinstance(wire, dict), 'live-continuation raw wire observation is missing')
        verified = model.validate_observation(entry, wire, stage)
        need(verified == observation,
             'live-continuation normalized timer/oracle differs from raw wire')
        return
    need(observation.get('mode') == mode and observation.get('result') == expected(entry), 'observation mode/oracle differs')
    if fuel_setting is not None:
        import p127_fuel_policy
        p127_fuel_policy.validate_wire_setting(observation, fuel_setting)
    if module == 'root_workloads.py':
        import root_compare
        root_compare.revalidate_observation(entry, observation)
        return
    if module == 'thread_workloads.py':
        import thread_compare
        thread_compare.revalidate(entry, observation, mode, stage, expected_thread_cpus=thread_cpus)
        return
    if module == 'gc_workloads.py':
        import gc_compare
        import gc_scaling
        import gc_workloads
        need(mode == 'decoded' and observation.get('stage') == 'load_and_call'
             and observation.get('batch') == 1 and observation.get('profile') == 'all-phase3-core',
             'legacy GC observation changed its typed cold timer identity')
        wire = observation.get('wire_observation')
        proof = gc_scaling._validate_row(entry, wire, focused_compare.input_value(entry), gc_workloads,
                                         entry['source']['sha256'], entry['source']['wasm_sha256'])
        need(proof == observation.get('validated_driver_proof'),
             'full GC host collection/root state oracle proof differs')
        # The retained observation has already converted the typed wire result
        # to the model value. Rebuild the raw adapter view for the wire-copy
        # check, then require the complete normalized observation to match.
        raw = dict(observation, result=wire.get('result'))
        normalized = gc_compare._normalize_cold(entry, raw, gc_workloads)
        need(normalized == observation, 'GC normalized oracle differs')
        names = ('setup_ns', 'checksum_ns') if entry['id'].startswith('gc_root_lifecycle_') else ('setup_ns', 'invoke_ns')
        components = {name:proof['driver_reported_times_ns'][name] for name in names}
        need(observation.get('cold_components_ns') == components
             and observation.get('ns_per_operation') == sum(components.values()),
             'GC cold metric differs from the validated driver intervals')
        return
    if module == 'simd_workloads.py':
        import simd_scaling
        import simd_workloads
        allowed = simd_scaling.allowed_wire_results(entry, simd_workloads)
        allowed_values = {int(value.removeprefix('i32:')) for value in allowed}
        need(type(observation.get('wire_result')) is int
             and observation['wire_result'] in allowed_values,
             'typed SIMD wire result differs from the independent allowed result set')
        limit = simd_scaling.batch_limit(entry)
        need(type(observation.get('batch')) is int and 1 <= observation['batch'] <= limit,
             f'typed SIMD batch must be an integer in 1..{limit}')
        if stage is not None:
            need(stage in entry.get('stages', []), 'typed SIMD stage is not declared for the selected row')
            if stage != 'warm_call':
                need(observation['batch'] == 1, 'typed SIMD cold stage is batched')
        timing = observation.get('ns_per_operation')
        need(not isinstance(timing, bool) and isinstance(timing, (int, float))
             and math.isfinite(timing) and timing > 0,
             'typed SIMD timing must be finite and positive')
        wanted_profile = focused_compare.profile(entry)
        wanted_settings = {'feature_profile': wanted_profile,
                           'features': entry.get('required_features'),
                           'fuel': False, 'cancellation_quantum': 1}
        need(observation.get('mode') == mode and observation.get('settings') == wanted_settings,
             'typed SIMD mode/profile/features/fuel/cancellation settings differ')
        if isinstance(entry.get('memory_expectation'), dict):
            arguments = entry.get('oracle_arguments', {})
            raw = next((arguments[key] for key in ('n', 'case', 'index', 'vectors', 'value')
                        if key in arguments), None)
            need(raw is not None, 'typed SIMD memory row lacks its oracle input')
            expected_memory = simd_workloads.expected_memory(entry['id'], raw)
            need(expected_memory.get('result') in allowed_values
                 and expected_memory.get('result') == expected(entry),
                 'typed SIMD memory and result oracles disagree')
            oracle_record = simd_scaling._memory_oracle_record(entry)
            if oracle_record is not None:
                oracle_record['result'] = expected_memory['result']
            need(observation.get('memory_oracle') == oracle_record,
                 'typed SIMD memory oracle record differs')
            page_size = expected_memory['page_size_bytes']
            need(type(page_size) is int and page_size > 0 and page_size & (page_size - 1) == 0,
                 'typed SIMD memory oracle page size is invalid')
            wanted_phase = 'preflight' if stage == 'prepare' else 'post_invoke'
            wanted_memory = {
                'memory_pages': expected_memory['page_count'],
                'memory_page_size_log2': page_size.bit_length() - 1,
                'memory_full_sha256': expected_memory['full_memory_sha256'],
                'memory_range': expected_memory['range'],
                'memory_range_sha256': expected_memory['sha256'],
                'memory_snapshot_phase': wanted_phase,
            }
            need({key: observation.get(key) for key in wanted_memory} == wanted_memory,
                 'typed SIMD full-memory/range/page/snapshot proof differs from oracle')
        return
    if module == 'memory_workloads.py':
        import memory_workloads
        exp = memory_workloads.expected_memory(entry['id'], focused_compare.input_value(entry))
        proof = observation.get('memory',{})
        need(proof.get('full_memory_sha256') == exp['full_memory_sha256']
             and proof.get('all_memory_sha256') == exp.get('all_memory_sha256'), 'memory oracle proof differs')
    if module == 'representative_workloads.py':
        import representative_workloads as model
        import representative_scaling as scaling
        import representative_compare as timer
        reference = observation.get('image_ref',{})
        relative = Path(reference.get('path',''))
        need(not relative.is_absolute() and '..' not in relative.parts
             and relative.parts[:1] == ('memory-images',), 'representative image path escapes evidence')
        path = (root/relative).resolve(strict=True)
        need(path.is_relative_to(root), 'representative image symlink escapes evidence')
        compressed = path.read_bytes()
        need(hashlib.sha256(compressed).hexdigest() == reference.get('compressed_sha256'), 'representative image changed')
        raw = gzip.decompress(compressed)
        need(len(raw) == reference.get('raw_bytes') and hashlib.sha256(raw).hexdigest() == reference.get('raw_sha256'),
             'representative raw image changed')
        wire = dict(observation['raw_wire_observation'],memory_hex=raw.hex())
        verified = timer.validate_observation(entry,wire,model,scaling,observation['batch'],expected_mode=mode,
            register_entry_threshold=observation.get('register_entry_threshold'))
        need(verified['validated_memory_proof'] == observation['validated_memory_proof'], 'representative image oracle differs')


def validate_semantics(path, declaration, current_sources):
    import semantic_receipt
    return semantic_receipt.validate(path, declaration, current_sources, focused_compare.ROOT)


def validate_fuel_setting(setting, entries, identities, capabilities):
    """Validate explicit P127 fuel policy bindings; False is the legacy wire form."""
    if setting is False:
        return None
    import p127_fuel_policy
    token = p127_fuel_policy.cli_value(setting)
    need(all(entry['oracle'].get('module') in ('scalar_workloads.py', 'run.py')
             or entry['id'] == 'prepare_large' for entry in entries),
         'explicit source fuel is supported only by scalar or single-case drivers')
    command_ref = identities.get('command')
    need(isinstance(command_ref, dict), 'explicit source fuel command identity is missing')
    command = json.loads(Path(command_ref['path']).read_text())
    argv = command.get('argv')
    index = argv.index('--source-fuel') if isinstance(argv, list) and '--source-fuel' in argv else -1
    need(index >= 0 and index + 1 < len(argv) and argv[index + 1] == token,
         'archived command lacks the exact source-fuel setting')
    for side in ('baseline', 'candidate'):
        need(capabilities[side].get('source_fuel_contract') == p127_fuel_policy.protocol_contract(),
             'both paired sides require the same explicit source-fuel contract')
    return setting


def validate_entry_observation(observation, threshold):
    import p140_entry_policy
    p140_entry_policy.validate_wire(observation, threshold)
    for key in ('wire_observation', 'raw_wire_observation'):
        if key in observation:
            p140_entry_policy.validate_wire(observation[key], threshold)


def validate_entry_setting(declaration, settings, identities, capabilities):
    """Bind the selected adaptive policy to its frozen value and actual CLI."""
    import p140_entry_policy
    entry_setting = p140_entry_policy.declaration(declaration)
    if entry_setting is None:
        need('register_entry_threshold' not in settings, 'undeclared adaptive entry setting')
    else:
        p140_entry_policy.validate_metadata(settings.get('register_entry_threshold'), entry_setting)
        p140_entry_policy.request(declaration, settings.get('candidate_mode'), entry_setting['candidate'])
        p140_entry_policy.capability(capabilities['candidate'].get('register_entry_threshold_contract'))
        need('command' in identities, 'adaptive entry command identity is missing')
    if 'command' in identities:
        command = json.loads(Path(identities['command']['path']).read_text())
        argv = command.get('argv')
        need(isinstance(argv, list), 'candidate command argv is missing')
        positions = [i for i, value in enumerate(argv) if value == '--register-entry-threshold']
        if entry_setting is None:
            need(not positions, 'undeclared adaptive entry command')
        else:
            need(len(positions) == 1 and positions[0] + 1 < len(argv)
                 and argv[positions[0] + 1] == str(entry_setting['candidate']),
                 'adaptive entry command differs from frozen selection')
    return entry_setting


def validate(path):
    path = Path(path).resolve(strict=True)
    report = json.loads(path.read_text())
    need(report.get('schema') == focused_compare.SCHEMA and report.get('lane') == 'candidate',
         'only full candidate evidence can reach the candidate gate')
    identities = report['identities']
    for identity in [*(value for key,value in identities.items()
                        if key not in ('fixture_files', 'setup_fixture_files')),
                     *identities.get('fixture_files',{}).values(),
                     *(ref for values in identities.get('setup_fixture_files',{}).values() for ref in values)]:
        need(compare.sha(identity['path']) == identity['sha256'], 'stale candidate artifact')
    encoder = report['metadata']['fixture_encoder']
    need(compare.sha(encoder['path']) == encoder['sha256'], 'stale encoder')
    declaration = selection_manifest.verify_frozen(Path(identities['selection_manifest']['path']),
                                                   Path(identities['selection_freeze']['path']))
    need(report['declaration'] == declaration and report['selection'] == declaration['task'],
         'comparison substituted its frozen selection')
    entries = selection_manifest.rows(declaration)
    by_id = {entry['id']:entry for entry in entries}
    settings = report['metadata']['settings']
    thread_selected = any(entry['oracle']['module'] == 'thread_workloads.py' for entry in entries)
    if thread_selected:
        import thread_compare
        thread_cpus = thread_compare.validate_cpu_affinity(settings.get('thread_cpu_affinity'))
        need(report['metadata'].get('hardware', {}).get('thread_cpu_affinity') == thread_cpus,
             'thread CPU affinity differs between settings and hardware metadata')
        command_ref = identities.get('command')
        need(isinstance(command_ref, dict), 'thread candidate command identity is missing')
        command = json.loads(Path(command_ref['path']).read_text())
        need(command.get('thread_cpu_affinity') == thread_cpus,
             'thread CPU affinity differs from archived command')
        argv = command.get('argv')
        mask_index = argv.index('--thread-cpus') if isinstance(argv, list) and '--thread-cpus' in argv else -1
        need(mask_index >= 0 and mask_index + 1 < len(argv)
             and argv[mask_index + 1] == ','.join(str(cpu) for cpu in thread_cpus),
             'archived command lacks the exact thread CPU mask')
    else:
        thread_cpus = None
    need('semantic_receipt' in identities, 'candidate semantic receipt is missing')
    validate_semantics(identities['semantic_receipt']['path'],declaration,focused_compare.sources())
    explicit_fuel = validate_fuel_setting(settings.get('fuel_policy'), entries, identities,
                                          report['metadata']['capabilities'])
    import p140_entry_policy
    entry_setting = validate_entry_setting(declaration, settings, identities,
                                          report['metadata']['capabilities'])
    need(settings.get('profile') == 'release' and settings.get('instrumented') is False
         and (settings.get('fuel_policy') is False or explicit_fuel == settings.get('fuel_policy'))
         and settings.get('cancellation_policy') == 1
         and settings.get('workload_profiles') == {entry['id']:focused_compare.profile(entry) for entry in entries},
         'candidate changed production/fuel/cancellation settings')
    for side in ('baseline','candidate'):
        need(identities[side]['sha256'] == report['metadata'][side+'_binary_sha256'], 'binary metadata differs')
    archive_path = Path(identities['baseline_manifest']['path']).parent
    archive = baseline.verify_archive(archive_path)
    info = compare._baseline_identity(archive_path, archive, focused_compare.HERE/'src/protocol.rs', baseline)
    freeze = json.loads(Path(identities['selection_freeze']['path']).read_text())
    predecessor = Path(archive['baseline']) if archive.get('kind') == 'frozen-typed-adapter' else archive_path
    need((predecessor/'manifest.json').resolve() == Path(freeze['predecessor_path'])
         and compare.sha(predecessor/'manifest.json') == freeze['predecessor_sha256'], 'predecessor substitution')
    need(compare.sha(info['executable']) == identities['baseline']['sha256']
         and info['mode'] == settings['baseline_mode'], 'baseline executable/mode differs')
    execution_identity.verify(report['metadata']['candidate_sources'], focused_compare.sources(), focused=True)
    need(set(identities.get('fixture_files',{})) == set(by_id), 'fixture inventory differs')
    for wid,entry in by_id.items():
        need(identities['fixture_files'][wid]['sha256'] == entry['source']['wasm_sha256'], 'fixture bytes differ from declaration')
    expected_setups = {wid:entry['setup_modules'] for wid,entry in by_id.items() if entry.get('setup_modules')}
    need(set(identities.get('setup_fixture_files',{})) == set(expected_setups),
         'setup fixture inventory differs')
    for wid, setups in expected_setups.items():
        refs = identities['setup_fixture_files'][wid]
        need(len(refs) == len(setups), 'setup fixture count differs')
        for reference, setup in zip(refs, setups):
            need(reference['sha256'] == setup['wasm_sha256'], 'setup fixture bytes differ from declaration')
    product_plan = cold_products.plan(focused_compare.products(entries, declaration, settings))
    cold_products.verify(report['cold_products'], focused_compare.products(entries, declaration, settings))
    metadata, runs = analyze._validated_runs(report)
    required = {(group['sample_workload'],group['stage']):group for group in product_plan['groups']}
    counts = {wid+'\0'+stage:0 for wid,stage in required}
    for run in runs:
        seen = set()
        for (wid,stage,_input,_fixture), pairs in run['groups'].items():
            need((wid,stage) in required and (wid,stage) not in seen, 'unexpected/duplicate candidate group')
            seen.add((wid,stage))
            need(len(pairs) == 30, 'candidate requires exactly 30 pairs per group per run')
            entry = by_id[wid]
            oracle = expected(entry)
            for pair in pairs:
                need(pair['expected'] == oracle and pair['wasm_sha256'] == entry['source']['wasm_sha256']
                     and pair['product_sha256'] == required[(wid,stage)]['product_sha256'],
                     'candidate oracle/fixture/product substitution')
                need(pair['input'] == focused_compare.input_value(entry),
                     'candidate input substitution')
                need(pair['baseline']['batch'] == pair['candidate']['batch'], 'unequal paired batches')
                for side in ('baseline','candidate'):
                    threshold = entry_setting[side] if entry_setting is not None else None
                    validate_entry_observation(pair[side], threshold)
                    check_observation(entry,pair[side],settings[side+'_mode'],path.parent,stage=stage,
                                      thread_cpus=thread_cpus, fuel_setting=explicit_fuel)
                    if (stage not in ('warm_call','run_after_setup')
                            and not (entry['oracle']['module'] == 'thread_workloads.py'
                                     and stage == 'steady_state_group')):
                        need(pair[side]['batch'] == 1, 'cold stage is batched')
                    if entry['oracle']['module'] == 'memory_workloads.py':
                        import memory_workloads
                        exp = memory_workloads.expected_memory(wid, pair['input'])
                        proof = pair[side].get('memory',{})
                        need(proof.get('full_memory_sha256') == exp['full_memory_sha256']
                             and proof.get('all_memory_sha256') == exp.get('all_memory_sha256'),
                             'memory oracle proof differs')
                if entry['oracle']['module'] == 'thread_workloads.py':
                    import thread_compare
                    need(thread_compare.paired_worker_identities_match(pair['baseline'], pair['candidate']),
                         'paired thread groups differ in effective worker count or quota')
            counts[wid+'\0'+stage] += len(pairs)
        need(seen == set(required), 'missing targeted/held-out/protected candidate groups')
    needed_probes = {(entry['id'],side) for entry in entries for side in ('baseline','candidate')}
    probes = report.get('correctness_probes',[])
    need(len(probes) == len(needed_probes) and {(row['workload'],row['side']) for row in probes} == needed_probes,
         'selected input correctness probes are incomplete')
    for row in probes:
        validate_entry_observation(row['observation'], entry_setting[row['side']] if entry_setting is not None else None)
        check_observation(by_id[row['workload']],row['observation'],settings[row['side']+'_mode'],
                          path.parent,stage=row.get('stage'),thread_cpus=thread_cpus,
                          fuel_setting=explicit_fuel)
        probe_entry = by_id[row['workload']]
        if probe_entry['oracle']['module'] == 'continuation_live_workloads.py':
            lifecycle = expected(probe_entry)
            wire = row['observation']['raw_wire_observation']
            need(row.get('timing_excluded') is True
                 and wire['payloads'] == lifecycle['payloads']
                 and wire['results'] == lifecycle['results'],
                 'untimed live-continuation lifecycle oracle differs')
        else:
            need(row.get('timing_excluded') is True
                 and row['observation']['result'] == expected(probe_entry),
                 'untimed input oracle differs')
    needed_calibrations = {(run['run_id'],wid,stage) for run in report['runs']
                           for wid,stage in required if stage in ('warm_call','run_after_setup')
                           or (by_id[wid]['oracle']['module'] == 'thread_workloads.py'
                               and stage == 'steady_state_group')}
    calibrations = report.get('warm_calibrations',[])
    need(len(calibrations) == len(needed_calibrations)
         and {(row['run_id'],row['workload'],row['stage']) for row in calibrations} == needed_calibrations,
         'warm calibration coverage differs')
    for row in calibrations:
        need(row.get('target_ns') == 20_000_000 and row['attempts'], 'calibration target differs')
        last = row['attempts'][-1]
        need(last['batch'] == row['batch'], 'calibration batch changed')
        for side in ('baseline','candidate'):
            observation = last['observations'][side]
            need(observation['batch'] == row['batch'] and observation['ns_per_operation']*row['batch'] >= 20_000_000,
                 'calibration did not actually reach 20 ms on both sides')
        for attempt in row['attempts']:
            for side in ('baseline','candidate'):
                validate_entry_observation(attempt['observations'][side], entry_setting[side] if entry_setting is not None else None)
                check_observation(by_id[row['workload']],attempt['observations'][side],settings[side+'_mode'],
                                  path.parent,stage=row['stage'],thread_cpus=thread_cpus,
                                  fuel_setting=explicit_fuel)
        raw_run = next(run for run in report['runs'] if run['run_id'] == row['run_id'])
        need(all(pair['candidate']['batch'] == row['batch'] for pair in raw_run['pairs']
                 if pair['workload'] == row['workload'] and pair['stage'] == row['stage']),
             'timed batch differs from calibration')
    need(report['counts'] == cold_products.summarize(product_plan, counts), 'sample counts changed')
    return report


def decision(report):
    """Analyze each measured product once; alias references share its interval."""
    summary = analyze.analyze(report)
    declaration = report['declaration']
    selected = {row['id']:row for row in declaration['rows']}
    targeted = {row['id'] for row in declaration['rows'] if row['role'] == 'targeted'}
    target_stages = {}
    for wid in targeted:
        declared = selected[wid]['stages']
        if 'warm_call' in declared:
            target_stages[wid] = 'warm_call'
        elif 'run_after_setup' in declared:
            target_stages[wid] = 'run_after_setup'
        elif selected[wid].get('family') == 'threads' and declared == ['steady_state_group']:
            target_stages[wid] = 'steady_state_group'
        elif (selected[wid].get('family') == 'continuations'
              and declared == ['setup', 'capture']):
            target_stages[wid] = 'capture'
        else:
            need(selected[wid].get('family') == 'gc-exceptions' and declared == ['load_and_call'],
                 'target needs a warm stage or the explicit legacy GC cold stage')
            target_stages[wid] = 'load_and_call'
    primary = []
    issues, inconclusive = [], []
    for number, run in enumerate(report['runs'],1):
        groups = {}
        for pair in run['pairs']:
            if (pair['workload'] in targeted
                    and pair['stage'] == target_stages[pair['workload']]):
                groups.setdefault(pair['workload'],[]).append(math.log(
                    pair['candidate']['ns_per_operation']/pair['baseline']['ns_per_operation']))
        need(set(groups) == targeted, 'every targeted row needs its declared primary observation')
        values = [sorted(rows) for _,rows in sorted(groups.items())]
        rng = random.Random(run['seed'])
        point = math.exp(sum(analyze._percentile(rows,.5) for rows in values)/len(values))
        draws = [math.exp(sum(draw_median(rows,rng) for rows in values)/len(values)) for _ in range(10_000)]
        upper = analyze._percentile(draws,.975)
        primary.append({'run':number,'median':point,'upper':upper,'lower':analyze._percentile(draws,.025),
                        'bootstrap_resamples':10_000,'weighting':'equal declared targeted workload weights',
                        'stages':{wid:target_stages[wid] for wid in sorted(targeted)}})
        if point > .95 or upper >= 1:
            inconclusive.append(f'run {number}: primary family has no supported 5% improvement')
    checked = []
    for row in summary['per_run']:
        role = selected[row['workload']]['role']
        stage = row['stage']
        limit = 1.25 if stage == 'prepare' else 1.05
        ratio = row['candidate_over_baseline']
        checked.append({'run':row['run'],'workload':row['workload'],'stage':stage,'limit':limit, 'ratio':ratio})
        if row['median_ratio_95ci']['lower'] > limit:
            issues.append(f"{row['workload']}/{stage}: supported regression")
        elif row['median_ratio_95ci']['upper'] > limit:
            inconclusive.append(f"{row['workload']}/{stage}: regression bound is inconclusive")
    return {'decision':'rejected' if issues else 'inconclusive' if inconclusive else 'adopted',
            'reasons':issues+inconclusive,'primary':primary,'controls':checked,
            'cold_products':report['cold_products'], 'sample_counts':report['counts'], 'analysis':summary}


def evaluate(task, path, build, optional_build, contract, candidate_build):
    import gates
    report = validate(path)
    need(report['declaration']['task'] == task, 'candidate task differs')
    need(contract and compare.sha(contract) == report['identities']['selection_manifest']['sha256'],
         'statistics must consume the same frozen selection manifest')
    task_record = gates._task(task)
    need(task_record['kind'] in ('candidate','experiment'), 'candidate evidence cannot complete a milestone')
    need(report['metadata']['settings']['candidate_mode'] == task_record['execution_mode'], 'task execution mode differs')
    need(candidate_build is not None, 'candidate production build attestation required')
    before = focused_compare.sources()
    snapshot = gates._make_build_snapshot()
    default = gates._validate_build(Path(build), expected_mode='default', label='default build', snapshot=snapshot)
    attestation = gates._validate_candidate_build_record(Path(candidate_build),report['metadata'],report)
    feature_build = None
    if attestation['runtime_features']:
        need(optional_build is not None, 'optional feature build required')
        feature_build = gates._validate_build(Path(optional_build),expected_mode='optional-feature',label='optional build',snapshot=snapshot)
    gates._validate_optional_features(feature_build, attestation['runtime_features'], optional_build)
    need(not gates._check_default_dependencies(snapshot), 'dependency budget failed')
    original = gates._read_json(gates.BASELINE_BUILD_PATH,'original build')
    growth = {'default':gates._relative_build_growth(default,original),
              'optional':gates._relative_build_growth(feature_build,default) if feature_build else {}}
    result = decision(report)
    if any(growth.values()) and result['decision'] == 'adopted':
        result['decision'] = 'inconclusive'
        result['reasons'].append('build/RSS/size growth requires a documented packaging decision')
    need(focused_compare.sources() == before, 'candidate sources changed during analysis')
    result.update({'task':task,'status':'passed','scope':'predeclared candidate only; no milestone or P001 total claim',
                   'build':{'attestation':attestation,'relative_growth':growth},'statistics_computations':1})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check-report',type=Path,required=True)
    args = parser.parse_args()
    report = validate(args.check_report)
    print(json.dumps({'status':'passed','task':report['selection'],'counts':report['counts'],'timings_collected':0}))


if __name__ == '__main__':
    main()
