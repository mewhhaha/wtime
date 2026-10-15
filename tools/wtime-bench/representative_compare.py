"""P035 staged representative paired comparator. UNBUILT/UNEXECUTED draft.

Uses the actual P015 independent scalar/output/full-memory oracle, fresh Store
per iteration, and only run_after_setup guest intervals. No Wasmtime speed claim.
"""
from __future__ import annotations
import hashlib
import gzip
import json
import math
import os
from pathlib import Path
import random

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
STAGE = 'run_after_setup'
PROFILE = 'mvp'
TARGET_NS = 20_000_000.0
MAX_ATTEMPTS = 8
MAX_BATCH = 100_000
MAX_PRODUCT = 50_000_000


class RepresentativeComparisonError(ValueError):
    pass


def need(condition, message):
    if not condition:
        raise RepresentativeComparisonError(message)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def validate_fixture_binding(wat, wasm, source):
    need(source.get('encoding_status') == 'encoded', 'fixture lacks actual encoding provenance')
    need(Path(wat).is_file() and Path(wasm).is_file(), 'fixture bytes missing')
    need(sha(wat) == source['sha256'] and sha(wasm) == source['wasm_sha256'], 'actual representative source/binary hash mismatch')


def cap(entry):
    size = entry['params'][0]['baseline']
    need(type(size) is int and size > 0, 'invalid representative model size')
    return min(MAX_BATCH, MAX_PRODUCT // size)


def validate_observation(entry, wire, model, scaling, batch, *, expected_mode='decoded', diagnostic=False, register_entry_threshold=None):
    need(type(batch) is int and 1 <= batch <= cap(entry), 'batch outside checked size/product bound')
    need(expected_mode in ('decoded', 'compact', 'register'), 'unsupported expected representative mode')
    need(not diagnostic or expected_mode != 'register', 'Register representative diagnostics are unsupported')
    if register_entry_threshold is not None:
        need(expected_mode == 'register' and not diagnostic, 'adaptive policy requires production Register timing')
    if register_entry_threshold is None:
        need(isinstance(wire, dict) and 'register_entry_threshold' not in wire, 'undeclared adaptive policy')
    else:
        import p140_entry_policy
        p140_entry_policy.validate_wire(wire, register_entry_threshold)
    need(isinstance(wire, dict) and wire.get('error') is None, 'invalid or failed representative wire')
    need(wire.get('batch') == batch and type(wire.get('batch')) is int, 'actual batch differs from request')
    need(wire.get('calls_checked') == batch and type(wire.get('calls_checked')) is int, 'each batch invocation must be checked')
    need(wire.get('report_complete') is True and wire.get('instrumented') is diagnostic, 'response complete/instrumentation mismatch')
    need(wire.get('mode') == expected_mode, 'representative wire mode differs from requested side mode')
    need(wire.get('setup_excluded_from_timing') is True, 'setup must be outside every guest interval')
    need(wire.get('setup_reset_before_each_call') is True, 'setup reset must precede each checked call')
    measured = wire.get('measured_guest_ns')
    if diagnostic:
        need(batch == 1 and wire.get('diagnostic') is True, 'diagnostic route requires explicit single-call identity')
        sample = wire.get('diagnostic_elapsed_ns')
    else:
        need(wire.get('diagnostic', False) is False, 'timing route cannot accept diagnostic evidence')
        sample = wire.get('ns_per_operation')
    need(type(measured) in (int,float) and math.isfinite(measured) and measured > 0, 'bad sum of actual guest intervals')
    need(type(sample) in (int,float) and math.isfinite(sample) and sample > 0, 'bad guest ns per invocation')
    need(math.isclose(sample, measured / batch, rel_tol=1e-9, abs_tol=0.000501), 'guest interval sum/batch mismatch')
    # Native P015 checker requires batch1. A COPY changes only that field to
    # reuse its full-memory/range/scalar proofs; actual batch/calls are checked
    # independently above. No timing result or byte image is rewritten.
    single = dict(wire, batch=1, ns_per_operation=sample)
    proof = scaling._validate_observation(single, entry, model, mode=expected_mode, profile=PROFILE)
    expected = model.oracle(entry['id'], entry['params'][0]['baseline'])
    return dict(wire, expected=expected['result'], result=expected['result'],
                raw_wire_observation=wire, validated_memory_proof=proof)


def retain_memory_image(observation, evidence_root):
    """Replace duplicated full-memory hex with one content-addressed gzip artifact.

    Call only after validate_observation has checked the complete image against
    the independent model.  The proof digest is checked again here so a caller
    cannot use this helper to preserve an unvalidated or mismatched image.
    """
    need(isinstance(observation, dict), 'validated observation must be an object')
    wire = observation.get('raw_wire_observation')
    proof = observation.get('validated_memory_proof')
    need(isinstance(wire, dict) and isinstance(proof, dict),
         'validated observation lacks its raw wire or memory proof')
    image_hex = wire.get('memory_hex')
    need(isinstance(image_hex, str), 'validated observation lacks the full memory image')
    try:
        image = bytes.fromhex(image_hex)
    except ValueError as exc:
        raise RepresentativeComparisonError('validated observation has malformed memory hex') from exc
    raw_digest = hashlib.sha256(image).hexdigest()
    need(proof.get('full_memory_sha256') == raw_digest,
         'validated memory proof does not bind the actual full image')
    need(len(image) > 0, 'validated full memory image is empty')

    root = Path(evidence_root).resolve()
    memory_dir = root / 'memory-images'
    memory_dir.mkdir(parents=True, exist_ok=True)
    artifact = memory_dir / f'{raw_digest}.bin.gz'
    try:
        artifact.resolve().relative_to(root)
    except ValueError as exc:
        raise RepresentativeComparisonError('memory image output escapes the evidence root') from exc
    compressed = gzip.compress(image, mtime=0)
    try:
        with artifact.open('xb') as stream:
            stream.write(compressed)
            stream.flush()
            os.fsync(stream.fileno())
    except FileExistsError:
        # Existing content is reusable only when both its decompressed bytes and
        # deterministic compressed representation match the requested image.
        try:
            existing = artifact.read_bytes()
            decoded = gzip.decompress(existing)
        except (OSError, EOFError, gzip.BadGzipFile) as exc:
            raise RepresentativeComparisonError('existing memory image artifact is corrupt') from exc
        if (decoded != image or hashlib.sha256(decoded).hexdigest() != raw_digest
                or existing != compressed):
            raise RepresentativeComparisonError('existing memory image artifact differs from its content identity')
    except OSError as exc:
        raise RepresentativeComparisonError(f'cannot retain full memory image: {exc}') from exc

    compressed_digest = hashlib.sha256(compressed).hexdigest()
    image_ref = {'path': f'memory-images/{artifact.name}',
                 'compressed_sha256': compressed_digest,
                 'raw_sha256': raw_digest, 'raw_bytes': len(image)}
    compact_wire = dict(wire)
    compact_wire.pop('memory_hex', None)
    compact_wire['image_ref'] = image_ref
    compact = dict(observation)
    compact.pop('memory_hex', None)
    compact['raw_wire_observation'] = compact_wire
    compact['image_ref'] = image_ref
    return compact


def calibrate(entry, observe):
    limit = cap(entry); batch = 1; attempts = []
    for index in range(1,MAX_ATTEMPTS+1):
        sides = []
        for side in ('baseline','candidate'):
            row = observe(side,batch)
            sides.append({'side':side,'wire':row['raw_wire_observation'],
                          'measured_guest_ns':row['measured_guest_ns'],
                          'sample_ns':row['ns_per_operation'],'calls_checked':row['calls_checked']})
        attempts.append({'attempt':index,'batch':batch,'sides':sides})
        if all(row['measured_guest_ns'] >= TARGET_NS for row in sides):
            return {'policy':'shared_exact_batch_both_sides_confirmed','target_ns':TARGET_NS,
                    'max_attempts':MAX_ATTEMPTS,'max_batch':limit,'paired_batch':batch,
                    'attempts':attempts,'sides':sides}
        if index == MAX_ATTEMPTS or batch == limit:
            break
        fastest = min(row['sample_ns'] for row in sides)
        projected = TARGET_NS / fastest * 1.1
        grown = limit if not math.isfinite(projected) or projected >= limit else math.ceil(projected)
        batch = min(limit,max(batch+1,grown))
    raise RepresentativeComparisonError('shared calibration did not actually reach target: '+json.dumps(attempts,allow_nan=False))


def run(args, growth_explanation=None):
    import baseline
    import checkpoint
    import compare
    import representative_workloads as model
    import representative_scaling as scaling
    import workloads
    need(args.selection == 'representative', 'wrong representative route')
    need(not getattr(args,'workload_subset',None), 'full representative9 selection cannot be silently subsetted')
    need(args.candidate_mode in ('decoded', 'compact') and type(args.pairs) is int and args.pairs >= 30,
         'requires decoded or compact candidate mode and at least30 pairs')
    checkpoint.Journal.preflight(args.output)
    manifest = baseline.verify_archive(args.baseline)
    info = compare._baseline_identity(args.baseline, manifest, HERE/'src/protocol.rs', baseline)
    need(info['kind'] in ('current-source-archive', 'frozen-typed-adapter') and info['mode'] == 'decoded',
         'representative route requires a source-bound decoded baseline archive')
    frozen = Path(info['frozen'])
    compare._verify_scalar_harness_sources(frozen,manifest)
    selected = workloads.selection('representative')
    expected_rows = model.entries()
    need(len(selected) == 9 and selected == expected_rows, 'full9 representative rows differ from independent source-bound model')
    candidate = Path(args.candidate).resolve(); before_sources = compare.sources(include_representative=True)
    before_binary = sha(candidate); baseline_exe = Path(info['executable']); before_baseline = sha(baseline_exe)
    available = sorted(os.sched_getaffinity(0)); cpu = args.cpu if args.cpu is not None else available[0]
    need(cpu in available, 'CPU outside actual process affinity')
    fixtures = {}
    files = manifest['files']
    for entry in selected:
        source = entry['source']; wat = HERE/source['path']; wasm = wat.with_suffix('.wasm')
        validate_fixture_binding(wat, wasm, source)
        for path,digest in ((wat,source['sha256']),(wasm,source['wasm_sha256'])):
            logical = path.relative_to(ROOT).as_posix(); record = files.get('source/'+logical)
            archived = frozen/'source'/logical
            need(isinstance(record,dict) and record.get('role') == 'source' and record.get('source_path') == logical
                 and record.get('sha256') == digest and archived.is_file() and sha(archived) == digest, 'archive does not bind actual representative fixture '+logical)
            need(before_sources.get(logical) == digest, 'candidate inventory does not bind actual representative fixture')
        fixtures[entry['id']] = {'path':wasm,'wat_sha256':source['sha256'],'wasm_sha256':source['wasm_sha256']}
    for logical,digest in before_sources.items():
        if logical.startswith('tools/wtime-bench/'):
            record=files.get('source/'+logical)
            need(isinstance(record,dict) and record.get('sha256')==digest and sha(frozen/'source'/logical)==digest,
                 'fresh baseline does not bind identical new measurement harness')
    capabilities = {side:compare.process(exe,['--protocol'],cpu) for side,exe in (('baseline',baseline_exe),('candidate',candidate))}
    for side, row in capabilities.items():
        selected_mode = info['mode'] if side == 'baseline' else args.candidate_mode
        need(row.get('schema_version') == 2 and row.get('instrumented') is False
             and selected_mode in row.get('modes',[]) and PROFILE in row.get('features',[]),
             f'actual protocol does not prove production {selected_mode}/MVP for {side}')
    settings = {'features':['mvp'],'feature_profiles':['mvp'],'fuel':False,'cancellation_quantum':1,
                'profile':'release','instrumented':False,'baseline_mode':info['mode'],'candidate_mode':args.candidate_mode,
                'representative_stage':STAGE,'representative_reset':'fresh Store/Instance per iteration; prepare before timed run',
                'representative_measurement_policy':'sum guest run intervals excluding compile/instantiate/reset/oracle/readback',
                'representative_calibration':{'target_ns':TARGET_NS,'max_attempts':MAX_ATTEMPTS,
                                             'max_batch':MAX_BATCH,'max_size_product':MAX_PRODUCT,
                                             'policy':'shared_exact_batch_both_sides_confirmed'},
                'wasmtime_pairing':'not attempted; existing P015 batch1 endpoint remains integrity-only'}
    metadata = {'settings':settings,'capabilities':capabilities,'hardware':{
                'selected_cpu':cpu,'cpu_affinity_available':available,
                'cpu_model':next((line.split(':',1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),'unknown'),
                'cpu_quota':Path('/sys/fs/cgroup/cpu.max').read_text().strip(),
                'physical_core_idle':'unknown/shared cloud'},
                'baseline_archive_kind':info['kind'],'baseline_archive_path':str(Path(args.baseline).resolve()),
                'baseline_manifest_sha256':sha(Path(args.baseline)/'manifest.json'),
                'frozen_manifest_sha256':sha(frozen/'manifest.json'),'baseline_binary_sha256':before_baseline,
                'candidate_binary_sha256':before_binary,'candidate_sources':before_sources,'protocol_sha256':sha(HERE/'src/protocol.rs'),
                'fixtures':[{'id':entry['id'],'wat_sha256':entry['source']['sha256'],'wasm_sha256':entry['source']['wasm_sha256'],
                             'input':entry['params'][0]['baseline'],'stages':entry['stages']} for entry in selected]}
    if growth_explanation is not None:metadata.update(growth_explanation['metadata'])
    seeds={run_id:args.seed+run_id*1000003 for run_id in (0,1)}
    identity={'selection':'representative','metadata':metadata,'pairs':args.pairs,'seed':args.seed}
    journal=checkpoint.Journal(args.output,identity,seeds)
    report={'schema_version':2,'selection':'representative','passed':False,'source_stable':False,
            'metadata':metadata,'runs':[],'warm_calibrations':[],'correctness_probes':[]}
    def observe(side,entry,batch):
        exe=baseline_exe if side=='baseline' else candidate
        n=entry['params'][0]['baseline']; expected=model.oracle(entry['id'],n)
        mode = info['mode'] if side == 'baseline' else args.candidate_mode
        row=compare.process(exe,['--representative-staged',fixtures[entry['id']]['path'],entry['id'],str(n),
                                f"i32:{expected['result']}",expected['output_sha256'],STAGE,str(batch),mode,PROFILE],cpu)
        validated = validate_observation(entry,row,model,scaling,batch,expected_mode=mode)
        return retain_memory_image(validated, args.output)
    try:
        for entry in selected:
            for side in ('baseline','candidate'):
                row=observe(side,entry,1)
                report['correctness_probes'].append({'case':entry['id'],'side':side,'status':'passed','timing_excluded':True,'observation':row})
        for run_id in (0,1):
            pairs=[];report['runs'].append({'run_id':run_id,'seed':seeds[run_id],'pairs':pairs})
            for entry in selected:
                n=entry['params'][0]['baseline']; expected=model.oracle(entry['id'],n)['result']
                cal=calibrate(entry,lambda side,batch:observe(side,entry,batch))
                report['warm_calibrations'].append({'run_id':run_id,'workload':entry['id'],'stage':STAGE,**cal})
                batch=cal['paired_batch']; rng=random.Random(seeds[run_id]+int(entry['source']['wasm_sha256'][:8],16))
                for sample in range(args.pairs):
                    order=['baseline','candidate'] if rng.getrandbits(1) else ['candidate','baseline']
                    pair={'pair_id':f"{run_id}:{entry['id']}:{STAGE}:{sample}",'workload':entry['id'],'stage':STAGE,
                          'input':n,'expected':expected,'family':entry['family'],'wat_sha256':entry['source']['sha256'],
                          'wasm_sha256':entry['source']['wasm_sha256'],'batch':batch,'order':order,
                          'wasmtime':{'status':'not_paired','reason':'existing batch1 integrity endpoint has no matched staged timer'}}
                    for side in order:pair[side]=observe(side,entry,batch)
                    journal.append(run_id,seeds[run_id],pair);pairs.append(pair)
        need(compare.sources(include_representative=True)==before_sources and sha(candidate)==before_binary and sha(baseline_exe)==before_baseline,'source/binary changed during paired run')
        compare._verify_build_growth_explanation(growth_explanation)
        need(len(report['warm_calibrations'])==18 and len(report['correctness_probes'])==18,'representative calibration/probe matrix incomplete')
        need(sum(len(r['pairs']) for r in report['runs'])==2*9*args.pairs,'paired representative matrix incomplete')
        report['counts']={'workloads':9,'groups_per_run':9,'groups_total':18,'pairs_per_group':args.pairs,
                          'pair_events':2*9*args.pairs,'side_observations':4*9*args.pairs,
                          'calibrations':18,'correctness_probes':18}
        report['passed']=True;report['source_stable']=True
    except Exception as error:
        report['error']=type(error).__name__+': '+str(error)
    finally:
        journal.close();journal.write_report_once(report)
    if not report['passed']:raise RepresentativeComparisonError(report['error'])
    return report
