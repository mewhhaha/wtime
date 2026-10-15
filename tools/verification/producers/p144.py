"""Source-bound Register working-vector capacity evidence; no guest timers."""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT/'tools'), str(ROOT/'tools/wtime-bench')]
from verification.artifacts import ArtifactStore, VerificationError, need, normal, read_json, verify_normal, write_new

ARCHIVE = ROOT/'target/performance/P144-before'
ARCHIVE_SHA = '94361e66c255210cbae5c44fd7086a89839868726b38b5e692b28ba363eb3c04'
PREDECESSOR = 'a3a3645a3017194af691aa4ff500790bb90d0042'
FIXTURE = ROOT/'crates/wtime-runtime/tests/fixtures/performance/p138/uniform66.wasm'
CASES = {'capacity_oracle', 'nested_oracle', 'pinned_eviction', 'entry_peaks', 'bounded_churn', 'reentry'}


def integer(value, label):
    need(type(value) is int and 0 <= value <= (1 << 64)-1, label+' must be a u64 integer')
    return value


def validate_metrics(rows):
    need(isinstance(rows, list) and len(rows) == 7, 'seven physical records are required')
    need(all(isinstance(row, dict) for row in rows), 'physical record must be an object')
    by = {}
    for row in rows:
        by.setdefault(row.get('case'), []).append(row)
    need(set(by) == CASES and all(len(items) == (2 if name == 'reentry' else 1)
                                for name, items in by.items()), 'missing or duplicate physical case')
    def exact(name, keys):
        row = by[name][0]
        need(set(row) == {'case', *keys}, 'unexpected physical fields: '+name)
        return row
    capacity = exact('capacity_oracle', {'before','after','capacity_not_length'})
    need(integer(capacity['after'], 'reserved bytes') > integer(capacity['before'], 'initial bytes') > 0
         and capacity['capacity_not_length'] is True, 'capacity oracle did not grow actual reservation')
    nested = exact('nested_oracle', {'first','second','sum','reset_preserved_active'})
    first, second, total = (integer(nested[key], key) for key in ('first','second','sum'))
    need(first > 0 and second > 0 and first+second == total and nested['reset_preserved_active'] is True,
         'constructed scope sum/reset oracle differs')
    pin = exact('pinned_eviction', {'working_bytes','code_bytes','evictions','cleanup'})
    need(0 < integer(pin['working_bytes'], 'pinned bytes') < integer(pin['code_bytes'], 'code bytes')
         and integer(pin['evictions'], 'pin evictions') >= 2 and pin['cleanup'] is True,
         'pinning did not keep code accounting separate')
    entries = exact('entry_peaks', {'peaks','per_invoke_reset'})
    peaks = entries['peaks']
    need(isinstance(peaks, list) and len(peaks) == 8, 'entry samples missing')
    for peak in peaks: integer(peak, 'entry peak')
    need(0 < peaks[0] <= 2*1024*1024 and peaks == [peaks[0]]*4+[0,0]+[peaks[0]]*2
         and entries['per_invoke_reset'] is True, 'cold/hot/reset sequence differs')
    churn = exact('bounded_churn', {'working_peak','code_bytes','evictions','cancel_peak'})
    need(integer(churn['working_peak'], 'churn peak') == peaks[0]
         and integer(churn['code_bytes'], 'churn code') > peaks[0]
         and integer(churn['evictions'], 'churn evictions') >= 3
         and integer(churn['cancel_peak'], 'cancel peak') == 0, 'churn/cancellation capacity differs')
    need({row.get('cancel') for row in by['reentry']} == {False, True}, 'both reentry outcomes required')
    for row in by['reentry']:
        need(set(row) == {'case','cancel','peak_frames','outer_released_before_callback','cleanup'}
             and type(row['cancel']) is bool and type(row['peak_frames']) is int and row['peak_frames'] == 1
             and row['outer_released_before_callback'] is True and row['cleanup'] is True,
             'real reentry must release outer vectors and restore scope')
    return by


def freeze(output):
    import baseline
    need(hashlib.sha256((ARCHIVE/'manifest.json').read_bytes()).hexdigest() == ARCHIVE_SHA, 'P144 archive changed')
    archive = baseline.verify_archive(ARCHIVE)
    need(archive['repository_commit'] == PREDECESSOR, 'P144 predecessor differs')
    sources = {row['source_path']:row['sha256'] for row in archive['files'].values()
               if row.get('role') == 'source' and ((row.get('source_path','').startswith('crates/')
               and (row['source_path'].endswith('.rs') or row['source_path'].endswith('/Cargo.toml')))
               or row.get('source_path') in ('Cargo.toml','Cargo.lock','rust-toolchain.toml'))}
    write_new(output, {'schema':'wtime-p144-freeze-v1','task':'P144','status':'passed',
        'predecessor':normal(ARCHIVE/'manifest.json'),'core_sources':sources,'timing_evidence':False})


def diagnose(binary, output):
    import entry_sequence_policy as policy
    need(not output.exists(), 'diagnostic directory already exists')
    output.mkdir(parents=True)
    shutil.copy2(binary, output/'diagnostic-bench')
    copied = output/'diagnostic-bench'
    binding = normal(copied)
    write_new(output/'inputs.json', {'binary':binding,'fixture':normal(FIXTURE),'instrumented':True,'timing_evidence':False})
    reports = {}
    for strategy, threshold in [('eager',0),('adaptive',3)]:
        reports[strategy] = {}
        for activation in (False, True):
            name = strategy+('-activation' if activation else '-cache')
            command = [str(copied), '--entry-sequence-activation-diagnostic' if activation else '--entry-sequence-diagnostic',
                       str(FIXTURE),'7','3','register',strategy,str(threshold)]
            write_new(output/(name+'.command.json'), {'argv':command,'cwd':str(ROOT)})
            with (output/(name+'.json')).open('x') as stdout, (output/(name+'.stderr')).open('x') as stderr:
                completed = subprocess.run(command,cwd=ROOT,stdout=stdout,stderr=stderr)
            write_new(output/(name+'.result.json'), {'exit_code':completed.returncode})
            need(completed.returncode == 0, 'diagnostic failed: '+name)
            payload = read_json(output/(name+'.json'))
            if activation:
                policy.validate_activation_payload(payload,3,strategy,threshold,7)
                cache = copy.deepcopy(payload)
                cache.pop('activation_capacity_basis')
                cache['schema'] = policy.SCHEMA
                cache['driver'] = 'entry_sequence'
                for row in cache['per_call']:
                    for field in list(row):
                        if field.startswith('activation_'): row.pop(field)
                need(cache == reports[strategy]['cache'], 'activation route changed old cache diagnostics')
            else:
                policy.validate_payload(payload,3,strategy,threshold,7,diagnostic=True)
            reports[strategy]['activation' if activation else 'cache'] = payload
    verify_normal(binding)
    write_new(output/'summary.json', {'status':'passed','timing_evidence':False,
        'binary':binding,'fixture':normal(FIXTURE),'calls_per_route':268,'routes':4,'legacy_schema_unchanged':True})


def summarize(verify_dir, freeze_path, output):
    import entry_sequence_policy as policy
    import p138
    frozen = read_json(freeze_path)
    need(frozen.get('schema') == 'wtime-p144-freeze-v1' and frozen.get('status') == 'passed', 'P144 freeze missing')
    verify_normal(frozen['predecessor'])
    receipt = read_json(verify_dir/'receipt.json')
    need(receipt['task'] == 'P144' and receipt['status'] == 'passed' and receipt['lane'] == 'full'
         and receipt['synthetic'] is False, 'P144 verify receipt incomplete')
    store = ArtifactStore(ROOT/'target/verification')
    for reference in receipt['artifacts'].values(): store.verify(reference)
    for reference in receipt['original_outputs'].values(): verify_normal(reference)
    logs = [verify_dir/'runtime-diagnostic.stdout', verify_dir/'performance-diagnostic.stdout']
    rows = [json.loads(line[len('P144_METRIC '):]) for path in logs for line in path.read_text().splitlines()
            if line.startswith('P144_METRIC ')]
    validate_metrics(rows)
    p138.validate_traces(logs)
    p138.validate_case_records(logs)
    diagnostic = verify_dir/'diagnostics'
    summary = read_json(diagnostic/'summary.json')
    need(summary.get('status') == 'passed' and summary.get('legacy_schema_unchanged') is True
         and summary.get('timing_evidence') is False, 'diagnostic routes incomplete')
    verify_normal(summary['binary']); verify_normal(summary['fixture'])
    observations = {}
    for strategy, threshold in [('eager',0),('adaptive',3)]:
        path = diagnostic/(strategy+'-activation.json')
        payload = read_json(path)
        policy.validate_activation_payload(payload,3,strategy,threshold,7)
        observations[strategy] = {'report':normal(path),'calls':payload['actual_calls'],
            'max_working_bytes':max(row['activation_peak_bytes'] for row in payload['per_call']),
            'max_code_bytes':max(row.get('retained_bytes',0) for row in payload['per_call']),
            'cold_calls':sum(row['activation_peak_bytes'] == 0 for row in payload['per_call'])}
    need(observations['eager']['max_working_bytes'] == observations['adaptive']['max_working_bytes'], 'same-shaped activation capacity differs')
    write_new(output, {'schema':'wtime-p144-activation-capacity-v1','task':'P144','status':'passed',
        'freeze':normal(freeze_path),'verification_receipt':normal(verify_dir/'receipt.json'),
        'tests':{name:value['passed'] for name,value in receipt['tests'].items()},
        'physical_oracles':rows,'entry_sequences':observations,'diagnostic_binary':summary['binary'],
        'timing_evidence':False,'candidate_promoted':False,'default_execution':'decoded',
        'capacity_basis':'slots.capacity() * size_of::<u64>() + labels.capacity() * size_of::<Label>()',
        'limitations':['Sampled live vector capacities, not allocator transient peaks, padding, generic frame locals, continuation payloads or process RSS.',
            'Current call handoff releases the caller Register activation; two-frame sum is a constructed private scope oracle, while real reentry reaches one live Register frame.',
            'All diagnostic storage and sampling require both register-executor and execution-reference; no default-build instrumentation.']})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('phase',choices=['freeze','diagnose','summarize'])
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--binary',type=Path)
    parser.add_argument('--verify-dir',type=Path)
    parser.add_argument('--freeze',type=Path)
    args = parser.parse_args()
    if args.phase == 'freeze': freeze(args.output)
    elif args.phase == 'diagnose': diagnose(args.binary,args.output)
    else: summarize(args.verify_dir,args.freeze,args.output)


if __name__ == '__main__': main()
