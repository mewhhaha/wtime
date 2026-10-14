"""Integrity regressions; synthetic fixtures only, isolated module copies."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest import mock
sys.dont_write_bytecode=True
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('staged_runner_fixture',HERE/'test_runner.py')
fixtures=importlib.util.module_from_spec(spec);spec.loader.exec_module(fixtures)

class IntegrityTests(unittest.TestCase):
    setUp=fixtures.WorkflowTests.setUp
    write=fixtures.WorkflowTests.write
    save=fixtures.WorkflowTests.save
    no_commands=fixtures.WorkflowTests.no_commands

    def specification(self):
        value=fixtures.WorkflowTests.specification(self)
        value['synthetic']=True
        value['completion']['required_phases']=['verify']
        return value

    def load(self,name):
        fullname=self.runner.__package__+'.'+name
        if fullname in sys.modules:return sys.modules[fullname]
        specification=importlib.util.spec_from_file_location(fullname,self.package_root/(name+'.py'))
        module=importlib.util.module_from_spec(specification);sys.modules[fullname]=module;specification.loader.exec_module(module);return module

    def test_delta_tightening_and_baseline_hash_cannot_reuse_success(self):
        artifacts=self.load('artifacts')
        baseline=self.repo/'baseline.json'
        self.write(baseline,json.dumps({'src/a.rs':artifacts.sha(self.repo/'src/a.rs')}))
        self.write(self.repo/'src/a.rs','// synthetic allowed runtime change\n')
        self.spec['source_delta']={'identity':'runtime','baseline':'{repo}/baseline.json','allowed_changes':['src/*.rs'],'allowed_additions':[]};self.save()
        self.workflow.run(self.spec_path,'verify')
        self.spec['source_delta']['allowed_changes']=[];self.save()
        with self.no_commands():
            with self.assertRaises(Exception):self.workflow.run(self.spec_path,'verify')
        self.spec['source_delta']['allowed_changes']=['src/*.rs'];self.save()
        self.write(baseline,json.dumps({'src/a.rs':'0'*64}))
        with self.no_commands():
            with self.assertRaises(Exception):self.workflow.run(self.spec_path,'verify')

    def test_same_matched_file_with_changed_glob_misses_cache(self):
        self.workflow.run(self.spec_path,'verify')
        self.spec['identities']['runtime']['paths']=['src/a.rs'];self.save()
        with self.assertRaises(Exception):
            with self.no_commands():self.workflow.run(self.spec_path,'verify')

    def test_declared_environment_only_invalidates_consuming_stage(self):
        self.spec['stages']['verify']['environment']=['SYNTHETIC_WORKFLOW_OPTION'];self.save()
        with mock.patch.dict(os.environ,{'SYNTHETIC_WORKFLOW_OPTION':'one'}):
            self.workflow.run(self.spec_path,'verify');self.workflow.run(self.spec_path,'freeze')
        with mock.patch.dict(os.environ,{'SYNTHETIC_WORKFLOW_OPTION':'two'}):
            with self.no_commands():self.workflow.run(self.spec_path,'freeze')
            with self.assertRaises(Exception):
                with self.no_commands():self.workflow.run(self.spec_path,'verify')

    def test_production_python_fake_cargo_rejected_before_execution(self):
        self.spec['synthetic']=False;self.save()
        with mock.patch('subprocess.run') as run,mock.patch('subprocess.Popen') as popen:
            with self.assertRaisesRegex(Exception,'Cargo|cargo'):self.workflow.run(self.spec_path,'verify')
            run.assert_not_called();popen.assert_not_called()

    def test_synthetic_completion_cannot_be_published(self):
        self.workflow.run(self.spec_path,'verify')
        with self.no_commands():final=self.workflow.finalize(self.spec_path,self.base/'synthetic-final')
        workspace=self.base/'publish-workspace';performance=workspace/'tasks/performance';repo=workspace/'wtime'
        task={'id':'P999','path':'P999.md','status':'todo','outcome':'pending','dependencies':[]}
        self.write(performance/'TASKS.json',json.dumps({'tasks':[task]}))
        self.write(performance/'INDEX.md','All 1 tasks are planned.\n')
        self.write(performance/'P999.md','Status: **todo**\nOutcome: **pending**\n\nNo commands in this planning file have been executed as implementation evidence.\n')
        self.write(performance/'check_plan.py',"STATUSES={'todo','in_progress','blocked','deferred','done'}\nOUTCOMES={'pending','contract_only','adopted','rejected','inconclusive'}\n")
        status=repo/'target/performance/TASK-status';status.mkdir(parents=True)
        original=(performance/'TASKS.json').read_bytes()
        publisher=self.load('publish')
        with mock.patch('subprocess.run') as run:
            self.assertEqual(publisher.execute(performance,Path(final['completion_record']),status/'synthetic-attempt'),1)
            attempt=json.loads((status/'synthetic-attempt/publication-attempt.json').read_text())
            self.assertFalse(attempt['completed']);self.assertIn('synthetic',attempt['error'])
            run.assert_not_called()
        self.assertEqual((performance/'TASKS.json').read_bytes(),original)

    def test_artifact_chmod_only_mutation_is_rejected(self):
        artifacts=self.load('artifacts');store=artifacts.ArtifactStore(self.base/'mode-store')
        source=self.write(self.repo/'payload.txt','synthetic immutable bytes\n');reference=store.add(source)
        object_path=store.root/reference['path'];os.chmod(object_path,0o644)
        with self.assertRaises(Exception):store.verify(reference)

    def statistics_fixture(self):
        statistics=self.load('statistics');artifacts=self.load('artifacts')
        self.write(self.repo/'tools/wtime-bench/execution_identity.py',
                   (HERE.parents[1]/'wtime-bench/execution_identity.py').read_text())
        self.statistics_paths={'src/a.rs','tools/wtime-bench/compare.py','tools/wtime-bench/gc_workloads.py','tools/wtime-bench/analyze.py','tools/wtime-bench/gates.py'}
        for relative in self.statistics_paths:
            path=self.repo/relative
            if not path.exists():self.write(path,'# synthetic source identity\n')
        def current(selection):
            self.assertEqual(selection,'integration')
            return {relative:artifacts.sha(self.repo/relative) for relative in sorted(self.statistics_paths)}
        measured=current('integration')
        report=self.write(self.repo/'report.json',json.dumps({'synthetic':True,'selection':'integration','metadata':{'candidate_sources':measured}}))
        build=self.write(self.repo/'build.json','{"synthetic":true}\n')
        result={'analysis':{'groups':{'synthetic':{'ratio':1.0}}},'task':'P999','decision':'rejected','reason':'synthetic protected regression'}
        evaluate=mock.Mock(side_effect=lambda *args:copy.deepcopy(result))
        def factory(*args):
            # bind_sources replaces the callable; each compute/audit loads a new gate.
            return SimpleNamespace(ROOT=self.repo,_current_source_identity=current,evaluate=evaluate)
        return statistics,report,build,result,evaluate,factory,current

    def test_statistics_compute_once_audit_full_decision_and_tamper(self):
        statistics,report,build,result,evaluate,factory,current=self.statistics_fixture()
        record=self.base/'statistics.json'
        with mock.patch.object(statistics,'module',side_effect=factory),self.no_commands():
            computed=statistics.compute(self.repo,'P999',report,build,None,None,None,record)
            self.assertEqual(evaluate.call_count,1)
            self.assertEqual(computed['analysis'],result['analysis']);self.assertEqual(computed['decision'],{k:v for k,v in result.items() if k!='analysis'})
            audit=statistics.audit(record,self.repo)
            self.assertTrue(audit['decision_recomputed']);self.assertEqual(evaluate.call_count,2)
            tampered=json.loads(record.read_text());tampered['decision']['reason']='decision-only tamper';self.write(record,json.dumps(tampered))
            with self.assertRaisesRegex(Exception,'decision'):statistics.audit(record,self.repo)
            self.assertEqual(evaluate.call_count,3)

    def test_statistics_analysis_only_changes_bind_same_report_once(self):
        statistics,report,build,result,evaluate,factory,current=self.statistics_fixture()
        before=report.read_bytes()
        for relative in statistics.ANALYSIS_ONLY:self.write(self.repo/relative,'# changed synthetic analysis-only source\n')
        with mock.patch.object(statistics,'module',side_effect=factory),self.no_commands():
            record=statistics.compute(self.repo,'P999',report,build,None,None,None,self.base/'analysis-only.json')
        self.assertEqual(evaluate.call_count,1);self.assertEqual(report.read_bytes(),before)
        binding=record['source_binding'];self.assertEqual(set(binding['analysis_only_changes']),statistics.ANALYSIS_ONLY)
        self.assertEqual(binding['measured'],json.loads(before)['metadata']['candidate_sources'])
        self.assertEqual(binding['analysis_inputs'],current('integration'))
        for relative,row in binding['analysis_only_changes'].items():
            self.assertEqual(row,{'measured_sha256':binding['measured'][relative],'analysis_sha256':binding['analysis_inputs'][relative]})

    def test_statistics_measured_input_hash_and_membership_drift_rejects_before_gate(self):
        statistics,report,build,result,evaluate,factory,current=self.statistics_fixture()
        original={relative:(self.repo/relative).read_text() for relative in self.statistics_paths}
        with mock.patch.object(statistics,'module',side_effect=factory),self.no_commands():
            for relative in ('src/a.rs','tools/wtime-bench/compare.py','tools/wtime-bench/gc_workloads.py'):
                with self.subTest(changed=relative):
                    self.write(self.repo/relative,'# changed synthetic measured input\n')
                    with self.assertRaisesRegex(Exception,'measured runtime'):statistics.compute(self.repo,'P999',report,build,None,None,None,self.base/'rejected.json')
                    self.write(self.repo/relative,original[relative])
            added='src/extra.rs';self.write(self.repo/added,'// synthetic membership addition\n');self.statistics_paths.add(added)
            with self.assertRaisesRegex(Exception,'membership'):statistics.compute(self.repo,'P999',report,build,None,None,None,self.base/'rejected.json')
            self.statistics_paths.remove(added);removed='tools/wtime-bench/compare.py';self.statistics_paths.remove(removed)
            with self.assertRaisesRegex(Exception,'membership'):statistics.compute(self.repo,'P999',report,build,None,None,None,self.base/'rejected.json')
        evaluate.assert_not_called();self.assertFalse((self.base/'rejected.json').exists())

    def test_statistics_source_drift_during_gate_rejects_publication(self):
        statistics,report,build,result,evaluate,factory,current=self.statistics_fixture()
        def drift(*args):
            self.write(self.repo/'src/a.rs','// synthetic drift during evaluation\n')
            return copy.deepcopy(result)
        evaluate.side_effect=drift
        with mock.patch.object(statistics,'module',side_effect=factory),self.no_commands():
            with self.assertRaisesRegex(Exception,'changed during evaluation'):statistics.compute(self.repo,'P999',report,build,None,None,None,self.base/'drifted.json')
        self.assertEqual(evaluate.call_count,1);self.assertFalse((self.base/'drifted.json').exists())

    def test_statistics_audit_wrong_repo_and_binding_tamper_rejects(self):
        statistics,report,build,result,evaluate,factory,current=self.statistics_fixture()
        record=self.base/'bound.json'
        with mock.patch.object(statistics,'module',side_effect=factory),self.no_commands():
            statistics.compute(self.repo,'P999',report,build,None,None,None,record)
            wrong=self.base/'wrong-repo'
            for name in ('gates.py','analyze.py'):self.write(wrong/'tools/wtime-bench'/name,(self.repo/'tools/wtime-bench'/name).read_text())
            with self.assertRaisesRegex(Exception,'producer paths differ'):statistics.audit(record,wrong)
            tampered=json.loads(record.read_text());tampered['source_binding']['analysis_inputs']['src/a.rs']='0'*64;self.write(record,json.dumps(tampered))
            with self.assertRaisesRegex(Exception,'source binding'):statistics.audit(record,self.repo)
        self.assertEqual(evaluate.call_count,1)

    def test_statistics_compute_and_audit_gate_root_mismatch_rejects(self):
        statistics,report,build,result,evaluate,factory,current=self.statistics_fixture()
        record=self.base/'valid.json';wrong=self.base/'wrong-gate-root';wrong.mkdir()
        with mock.patch.object(statistics,'module',side_effect=factory),self.no_commands():statistics.compute(self.repo,'P999',report,build,None,None,None,record)
        def wrong_factory(*args):
            gate=factory();gate.ROOT=wrong;return gate
        with mock.patch.object(statistics,'module',side_effect=wrong_factory),self.no_commands():
            with self.assertRaisesRegex(Exception,'workspace differs'):statistics.compute(self.repo,'P999',report,build,None,None,None,self.base/'wrong.json')
            with self.assertRaisesRegex(Exception,'workspace differs'):statistics.audit(record,self.repo)
        self.assertEqual(evaluate.call_count,1)

    def test_cached_output_rules_revalidated_and_finalize_no_commands(self):
        self.workflow.run(self.spec_path,'verify')
        cache_files=list((self.workflow.root/'cache/P999/verify').glob('*.json'));self.assertEqual(len(cache_files),1)
        cache=cache_files[0];reference=json.loads(cache.read_text());receipt=self.workflow.store.json(reference)
        wrong=self.write(self.repo/'wrong-output.json',json.dumps({'synthetic':True,'passed':True,'work':0,'profile':'compact'}))
        receipt['artifacts']['result']=self.workflow.store.add(wrong)
        altered=self.write(self.repo/'altered-receipt.json',json.dumps(receipt));altered_ref=self.workflow.store.add(altered)
        self.write(cache,json.dumps(altered_ref))
        self.write(self.workflow._state_path('P999','verify'),json.dumps(altered_ref))
        with mock.patch('subprocess.run') as run,mock.patch('subprocess.Popen') as popen:
            with self.assertRaises(Exception):self.workflow.run(self.spec_path,'verify')
            with self.assertRaises(Exception):self.workflow.finalize(self.spec_path,self.base/'bad-final')
            run.assert_not_called();popen.assert_not_called()


    def chain(self):
        artifacts=self.load('artifacts')
        self.write(self.repo/'predecessor/a.rs','// frozen synthetic predecessor\n')
        self.write(self.repo/'src/a.rs','// frozen synthetic predecessor\n')
        script=r"""import argparse,hashlib,json,pathlib
p=argparse.ArgumentParser();p.add_argument('phase');p.add_argument('output');p.add_argument('input')
a=p.parse_args();source=pathlib.Path(a.input)
if a.phase=='freeze':value={'src/a.rs':hashlib.sha256(source.read_bytes()).hexdigest()}
else:
 prior=json.loads(source.read_text());assert prior
 value={'synthetic':True,'passed':True,'work':1,'profile':'compact','consumed':str(source)}
pathlib.Path(a.output).write_text(json.dumps(value))
"""
        self.write(self.repo/'chain.py',script)
        self.spec['identities']['predecessor']={'paths':['predecessor/*.rs'],'settings':{}}
        freeze={'uses':['predecessor'],'dependencies':[],
                'commands':[{'name':'freeze','argv':['{python}','{repo}/chain.py','freeze','{stage}/baseline.json','{repo}/predecessor/a.rs']}],
                'outputs':{'baseline':'{stage}/baseline.json'},'checks':[{'artifact':'baseline','pointer':'/src~1a.rs','op':'equal','value':artifacts.sha(self.repo/'predecessor/a.rs')}],'tests':[]}
        stages={'freeze':freeze}
        for phase,parent,deps in [('verify','freeze',['freeze']),('measure','verify',['freeze','verify']),('analyze','measure',['freeze','measure'])]:
            prior='{'+parent+'_dir}/'+('baseline.json' if parent=='freeze' else 'result.json')
            stages[phase]={'uses':['runtime'],'dependencies':deps,
                'commands':[{'name':'produce','argv':['{python}','{repo}/chain.py',phase,'{stage}/result.json',prior]}],
                'outputs':{'result':'{stage}/result.json'},'checks':[{'artifact':'result','pointer':'/passed','op':'equal','value':True}],'tests':[]}
        self.spec['stages']=stages
        self.spec['source_delta']={'identity':'runtime','baseline':'{freeze_dir}/baseline.json','allowed_changes':['src/a.rs'],'allowed_additions':[]}
        self.spec['completion']['required_phases']=['freeze','verify','measure','analyze'];self.save()

    def test_dependency_macros_frozen_baseline_survives_candidate_edit(self):
        self.chain();freeze=self.workflow.run(self.spec_path,'freeze')
        self.write(self.repo/'src/a.rs','// synthetic candidate edit\n')
        with self.no_commands():self.workflow.run(self.spec_path,'freeze')
        receipts={phase:self.workflow.run(self.spec_path,phase) for phase in ('verify','measure','analyze')}
        self.assertEqual(receipts['verify']['source_delta']['changed'],['src/a.rs'])
        artifacts=self.load('artifacts')
        for phase in ('verify','measure','analyze'):
            receipt=receipts[phase]
            produced=self.workflow.store.json(receipt['artifacts']['result'])
            expected_parent='freeze' if phase=='verify' else ('verify' if phase=='measure' else 'measure')
            parent=freeze if expected_parent=='freeze' else receipts[expected_parent]
            self.assertTrue(Path(produced['consumed']).is_relative_to(Path(parent['attempt'])))
        with self.no_commands():
            for phase in ('freeze','verify','measure','analyze'):self.workflow.run(self.spec_path,phase)
            self.workflow.finalize(self.spec_path,self.base/'chain-final')

    def test_original_dependency_output_tamper_rejected_before_commands(self):
        self.chain();freeze=self.workflow.run(self.spec_path,'freeze')
        raw=Path(freeze['original_outputs']['baseline']['path'])
        self.write(raw,'{"src/a.rs":"tampered raw original"}\n')
        with mock.patch('subprocess.run') as run,mock.patch('subprocess.Popen') as popen:
            with self.assertRaises(Exception):self.workflow.run(self.spec_path,'verify')
            run.assert_not_called();popen.assert_not_called()

    def test_parent_semantic_identity_change_makes_cached_consumers_stale(self):
        self.chain()
        for phase in ('freeze','verify','measure','analyze'):self.workflow.run(self.spec_path,phase)
        self.spec['identities']['predecessor']['settings']['semantic_revision']='synthetic changed predecessor';self.save()
        with mock.patch('subprocess.run') as run,mock.patch('subprocess.Popen') as popen:
            for phase in ('verify','measure','analyze'):
                with self.assertRaisesRegex(Exception,'stale'):self.workflow.run(self.spec_path,phase)
            run.assert_not_called();popen.assert_not_called()

    def test_script_named_cargo_is_not_the_selected_cargo(self):
        artifacts=self.load('artifacts')
        fake=self.write(self.repo/'fake/bin/cargo','#!/bin/sh\necho synthetic fake Cargo\n');os.chmod(fake,0o700)
        selected=self.repo/'selected/bin/cargo'
        self.assertTrue(selected.resolve(strict=False).is_relative_to(self.base));selected.parent.mkdir(parents=True);selected.write_bytes(b'\x7fELFexplicitly synthetic selected toolchain stub, never executed');os.chmod(selected,0o700)
        environment={'variables':{},'toolchain':{str(selected):artifacts.sha(selected)}}
        self.spec['synthetic']=False
        self.spec['stages']['verify']['commands'][0]['argv']=[str(fake),'test'];self.save()
        with mock.patch.object(self.runner,'execution_environment',return_value=environment),mock.patch('subprocess.run') as run,mock.patch('subprocess.Popen') as popen:
            with self.assertRaisesRegex(Exception,'selected toolchain Cargo'):self.workflow.run(self.spec_path,'verify')
            run.assert_not_called();popen.assert_not_called()

if __name__=='__main__':unittest.main()
