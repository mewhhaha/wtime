"""P033 compare adapter for the existing one-shot GC/exception cold rows.

This module is a bounded tooling adapter. In its eventual compare.py integration,
process and ComparisonError resolve to compare.py's existing helpers.
"""
from __future__ import annotations
import math
from typing import Any

GC_COLD_PROFILE = 'all-phase3-core'
GC_COLD_MODE = 'decoded'


def cold_observation(exe, fixture, entry, n, stage, batch, mode, cpu):
    """Run and validate one existing load_and_call GC/exception observation.

    fixture is {"path": WasmPath, "setup_modules": [providerWasmPath, ...]}.
    The independent model and gc_scaling validator provide all semantic proof.
    """
    import gc_scaling
    import gc_workloads

    workload=entry.get('id')
    if not isinstance(workload,str) or not workload:
        raise ComparisonError('GC cold entry has no workload id')
    if stage != 'load_and_call' or type(batch) is not int or batch != 1:
        raise ComparisonError(f'{workload}: legacy GC cold rows require load_and_call with batch 1')
    if mode != GC_COLD_MODE:
        raise ComparisonError(f'{workload}: legacy GC cold rows require decoded mode')
    try:
        expected_n=gc_scaling._expected_input(entry)
    except Exception as exc:
        raise ComparisonError(f'{workload}: invalid GC cold workload input: {exc}') from exc
    if type(n) is not int or n != expected_n:
        raise ComparisonError(f'{workload}: input differs from fixed GC workload model')
    if not isinstance(fixture,dict) or not fixture.get('path'):
        raise ComparisonError(f'{workload}: fixture must provide an encoded Wasm path')
    path=str(fixture['path'])
    setup=fixture.get('setup_modules',[])
    if not isinstance(setup,list) or any(not item for item in setup):
        raise ComparisonError(f'{workload}: setup_modules must be encoded Wasm paths')

    is_cross=workload.startswith('gc_descriptor_cross_module_')
    if is_cross:
        if len(setup)!=1:
            raise ComparisonError(f'{workload}: cross-module fixture needs exactly one provider Wasm')
        args=['--gc-exception-cross-module',str(setup[0]),path,f'i32:{n}',mode,GC_COLD_PROFILE]
    else:
        if setup:
            raise ComparisonError(f'{workload}: single-module fixture must not provide setup modules')
        args=['--gc-exception-driver',path,workload,f'i32:{n}',mode,GC_COLD_PROFILE]
    row=process(exe,args,cpu)
    try:
        oracle=gc_workloads.oracle(workload,n)
        source=entry.get('source',{})
        expected_wat=source.get('sha256')
        expected_wasm=source.get('wasm_sha256')
        if not expected_wat or not expected_wasm:
            raise gc_scaling.MeasurementError(f'{workload}: fixture hashes missing from independent model')
        validated=gc_scaling._validate_row(entry,row,n,gc_workloads,expected_wat,expected_wasm)
    except Exception as exc:
        raise ComparisonError(f'{workload}: GC cold semantic validation failed: {exc}') from exc

    # Keep all original driver timings and full state proof. The cold metric
    # deliberately excludes explicit host collection and root-lifecycle extras.
    components=validated['driver_reported_times_ns']
    if workload.startswith('gc_root_lifecycle_'):
        names=('setup_ns','checksum_ns')
    else:
        names=('setup_ns','invoke_ns')
    cold_ns=sum(components[name] for name in names)
    if not math.isfinite(cold_ns) or cold_ns <= 0:
        raise ComparisonError(f'{workload}: derived GC cold interval is invalid')
    return {
        'workload':workload,'stage':stage,'input':n,'expected':oracle,
        'result':row.get('result'),'outcome':row.get('outcome','returned'),
        'tag':row.get('tag'),'payload':row.get('payload'),
        'ns_per_operation':cold_ns,'batch':1,'mode':mode,'profile':GC_COLD_PROFILE,
        'driver':validated['driver'],'fixture':validated['fixture'],
        'cold_components_ns':{name:components[name] for name in names},
        'driver_reported_times_ns':components,
        'validated_driver_proof':validated,
        'wire_observation':row,
    }
