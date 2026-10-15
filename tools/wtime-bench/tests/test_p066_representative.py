"""Contained tests for P066 representative Register mode bindings."""
from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
BENCH = Path(__file__).resolve().parents[1]
ROOT = BENCH.parents[1]
sys.path.insert(0, str(BENCH))

import focused_compare
import p140_entry_policy
import representative_compare
import representative_scaling
import representative_workloads


def inside(root: Path, path: Path) -> Path:
    root = root.resolve()
    resolved = path.resolve()
    if not resolved.is_relative_to(root):
        raise AssertionError(f'synthetic path escapes fresh temporary root: {resolved}')
    return resolved


class P066RepresentativeTests(unittest.TestCase):
    def test_p066_probe_and_calibration_nested_policy_cannot_differ(self):
        import candidate_gate
        for key in ('wire_observation', 'raw_wire_observation'):
            good = {'register_entry_threshold': 3, key: {'register_entry_threshold': 3}}
            candidate_gate.validate_entry_observation(good, 3)
            for bad in (None, 2, True, 3.0):
                nested = {} if bad is None else {'register_entry_threshold': bad}
                with self.assertRaises(ValueError):
                    candidate_gate.validate_entry_observation(dict(good, **{key: nested}), 3)
            with self.assertRaises(ValueError):
                candidate_gate.validate_entry_observation(good, None)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='p066-representative-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        audit = inside(self.root, self.root / 'audit')
        audit.mkdir()
        for source in (Path(__file__), BENCH/'focused_compare.py', BENCH/'representative_compare.py',
                       BENCH/'representative_scaling.py', BENCH/'representative_workloads.py',
                       BENCH/'p140_entry_policy.py'):
            copy = inside(self.root, audit/source.name)
            copy.write_bytes(source.read_bytes())
        invocation = inside(self.root, audit/'invocation.json')
        invocation.write_text(json.dumps({'test': self.id(), 'argv': list(sys.argv)}, sort_keys=True))
        self.output = inside(self.root, self.root/'output')
        self.output.mkdir()
        self.entry = representative_workloads.entries()[0]
        self.size = self.entry['params'][0]['baseline']
        self.oracle = representative_workloads.oracle(self.entry['id'], self.size)

    def wire(self, mode='register', threshold=None):
        value = {
            'schema_version': 2, 'case': self.entry['id'], 'stage': 'run_after_setup',
            'input': str(self.size), 'expected': f"i32:{self.oracle['result']}",
            'result': f"i32:{self.oracle['result']}", 'batch': 4, 'calls_checked': 4,
            'report_complete': True, 'setup_reset_before_each_call': True,
            'setup_excluded_from_timing': True, 'instrumented': False,
            'measured_guest_ns': 400.0, 'ns_per_operation': 100.0,
            'mode': mode, 'profile': 'mvp', 'memory_pages': 1,
            'memory_page_size_log2': 16,
            'memory_hex': representative_workloads.expected_memory_image(
                self.entry['id'], self.size).hex(),
            'expected_output_sha256': self.oracle['output_sha256'],
        }
        if threshold is not None:
            value['register_entry_threshold'] = threshold
        return value

    def validate(self, wire, *, mode='register', threshold=None):
        return representative_compare.validate_observation(
            self.entry, wire, representative_workloads, representative_scaling, 4,
            expected_mode=mode, register_entry_threshold=threshold)

    def test_p066_representative_forwards_adaptive_threshold_to_candidate_only(self):
        fixture = inside(self.root, self.root/'synthetic-fixture.wasm')
        fixture.write_bytes(b'synthetic fixture identity; subprocess is mocked')
        calls = []

        def process(_exe, argv, _cpu):
            calls.append(list(argv))
            mode = argv[8]
            threshold = int(argv[-1]) if argv[-2] == '--register-entry-threshold' else None
            return self.wire(mode, threshold)

        with mock.patch.object(focused_compare.compare, 'process', side_effect=process):
            eager = focused_compare.observe('baseline', self.entry, str(fixture), 'run_after_setup',
                4, 'register', 0, self.output, register_entry_threshold=None)
            adaptive = focused_compare.observe('candidate', self.entry, str(fixture), 'run_after_setup',
                4, 'register', 0, self.output, register_entry_threshold=3)
        self.assertNotIn('--register-entry-threshold', calls[0])
        self.assertEqual(calls[1][-2:], ['--register-entry-threshold', '3'])
        self.assertNotIn('register_entry_threshold', eager['raw_wire_observation'])
        self.assertEqual(adaptive['raw_wire_observation']['register_entry_threshold'], 3)
        self.assertEqual(adaptive['validated_memory_proof']['full_memory_sha256'],
                         self.entry['expected_full_memory_sha256'])

    def test_p066_wire_threshold_and_full_memory_tampering_are_rejected(self):
        valid = self.wire(threshold=3)
        checked = self.validate(valid, threshold=3)
        self.assertEqual(checked['validated_memory_proof']['full_memory_sha256'],
                         self.entry['expected_full_memory_sha256'])
        for bad in ({**valid, 'register_entry_threshold': 2},
                    {key: value for key, value in valid.items()
                     if key != 'register_entry_threshold'}):
            with self.subTest(threshold=bad.get('register_entry_threshold')):
                with self.assertRaises((ValueError, representative_compare.RepresentativeComparisonError)):
                    self.validate(bad, threshold=3)
        tampered = dict(valid)
        image = bytearray.fromhex(tampered['memory_hex'])
        image[-1] ^= 1
        tampered['memory_hex'] = image.hex()
        with self.assertRaises(representative_scaling.RepresentativeError):
            self.validate(tampered, threshold=3)

    def test_p066_omitted_threshold_keeps_legacy_wire_for_decoded_and_eager_register(self):
        for mode in ('decoded', 'register'):
            wire = self.wire(mode=mode)
            self.assertNotIn('register_entry_threshold', wire)
            self.assertIs(p140_entry_policy.validate_wire(wire, None), wire)
            checked = self.validate(wire, mode=mode, threshold=None)
            self.assertEqual(checked['validated_memory_proof']['full_memory_sha256'],
                             self.entry['expected_full_memory_sha256'])
        bad_legacy = self.wire(mode='register', threshold=3)
        with self.assertRaises(ValueError):
            p140_entry_policy.validate_wire(bad_legacy, None)


if __name__ == '__main__':
    unittest.main()
