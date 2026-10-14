import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest
import tempfile

ROOT=Path(__file__).resolve().parents[3]
spec=importlib.util.spec_from_file_location('opcode_coverage',ROOT/'tools/checks/opcode_coverage.py')
coverage=importlib.util.module_from_spec(spec)
spec.loader.exec_module(coverage)

class OpcodeLedgerTests(unittest.TestCase):
    def setUp(self):
        self.ledger=json.loads((ROOT/'docs/architecture/opcodes.json').read_text())
        # Schema tests are isolated from concurrent implementation changes.
        # Production check_ledger still rejects stale recorded source evidence.
        for row in self.ledger['instructions']+self.ledger.get('non_opcode_features',[]):
            for owner in row['owners'].values():
                evidence=owner.get('evidence',{})
                if evidence.get('kind')=='source-handler':
                    evidence['source_sha256']=hashlib.sha256((ROOT/evidence['path']).read_bytes()).hexdigest()
                elif evidence.get('kind') in ('executed-corpus','corpus-report','focused-tests'):
                    owner['status']='pending-audit'
                    owner.pop('evidence',None)

        self.scope={s['feature_id'] for s in json.loads((ROOT/'docs/architecture/spec-lock.json').read_text())['sources']}-{'proposal-catalog'}
    def test_t008_case_01_missing_owner(self):
        del self.ledger['instructions'][0]['owners']['execute']
        errors,_=coverage.check_ledger(self.ledger,self.scope,verify_freshness=False)
        self.assertTrue(any('missing execute task owner' in e for e in errors))
    def test_t008_case_02_reserved_handler(self):
        reserved=next(coverage.reserved_keys(self.ledger))
        assigned={r['encoding'] for r in self.ledger['instructions']}
        errors,_=coverage.check_ledger(self.ledger,self.scope,probe=lambda key:key in assigned or key==reserved,verify_freshness=False)
        self.assertTrue(any('reserved opcode acquired' in e for e in errors))
    def test_t008_case_03_non_opcode_rows(self):
        errors,pending=coverage.check_ledger(self.ledger,self.scope,verify_freshness=False)
        self.assertEqual(errors,[])
        self.assertTrue(pending,'unfinished semantics must remain explicit')
        self.ledger['non_opcode_features']=[r for r in self.ledger['non_opcode_features'] if r['feature']!='compact-import-section']
        errors,_=coverage.check_ledger(self.ledger,self.scope,verify_freshness=False)
        self.assertIn('missing non-opcode feature: compact-import-section',errors)
    def test_t008_missing_actual_decoder(self):
        absent=self.ledger['instructions'][0]['encoding']
        assigned={r['encoding'] for r in self.ledger['instructions']}
        errors,_=coverage.check_ledger(self.ledger,self.scope,probe=lambda key:key in assigned and key!=absent,verify_freshness=False)
        self.assertIn('missing decoder handler: '+absent,errors)
    def test_t008_missing_symbol_evidence(self):
        self.ledger['instructions'][0]['owners']['decode']['symbol']='nonexistent_handler_symbol'
        errors,_=coverage.check_ledger(self.ledger,self.scope,verify_freshness=False)
        self.assertTrue(any('missing implemented decode handler' in e for e in errors))

    def test_t169_stale_source_evidence_blocks_release(self):
        self.ledger['instructions'][0]['owners']['decode']['evidence']['source_sha256']='0'*64
        errors,_=coverage.check_ledger(self.ledger,self.scope,verify_freshness=True)
        self.assertTrue(any('source changed since evidence audit' in e for e in errors))

    def test_t169_premise_inspection_without_tests_remains_pending(self):
        rule=self.ledger['validation_rules'][0]
        rule.update({'status':'pending-premise-audit','premise_evidence':[{'inspection':'real operand checks'}],'test_evidence':[]})
        errors,pending=coverage.check_ledger(self.ledger,self.scope,verify_freshness=False)
        self.assertIn(rule['id']+':premise-audit',pending)
        rule['status']='audited'
        errors,_=coverage.check_ledger(self.ledger,self.scope,verify_freshness=False)
        self.assertIn('audited rule lacks premise/test proof: '+rule['id'],errors)

    def test_t169_corpus_span_uses_wast_lexical_boundaries(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'case.wast'
            form=b'(module (data "(ignore)") (; nested (; ( ;) ;) )'
            path.write_bytes(b';; preface\n'+form+b'\n(module)')
            self.assertEqual(coverage.case_form_hash(path,11),hashlib.sha256(form).hexdigest())
            self.assertIsNone(coverage.case_form_hash(path,0))
            path.write_bytes(b'(; '+form+b' ;)')
            self.assertIsNone(coverage.case_form_hash(path,3))

    def test_t169_failed_focused_tests_cannot_discharge(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'focused.json'
            report={'passed':False,'source_stable':True,'exit_code':1,'executed':1,'command':['cargo','test'],'output':'test named ... FAILED'}
            path.write_text(json.dumps(report))
            owner=self.ledger['non_opcode_features'][0]['owners']['test']
            owner.update({'status':'implemented','evidence':{'kind':'focused-tests','reports':[{'path':str(path),'report_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'command':['cargo','test'],'tests':['named']}]}})
            errors,_=coverage.check_ledger(self.ledger,self.scope,verify_freshness=False)
            self.assertTrue(any('focused tests not actually passed' in error for error in errors))

    def test_t169_missing_formal_rule_blocks_release(self):
        self.ledger['validation_rules'].pop()
        errors,_=coverage.check_ledger(self.ledger,self.scope,verify_freshness=False)
        self.assertIn('formal validation rule inventory is missing or changed',errors)

if __name__=='__main__':unittest.main()
