"""Stdlib workflow tests using explicitly synthetic output and confined temp roots.

Staged tests: do not run until root implementation is available. No Rust/native
fixture evidence is produced. Every write resolves inside its fresh case root.
"""
import copy
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest import mock
import uuid
sys.dont_write_bytecode = True
MODULE_SOURCE = Path(__file__).resolve().parents[1]

PRODUCER = r'''import argparse,json,pathlib
p=argparse.ArgumentParser()
p.add_argument('output');p.add_argument('--profile',default='compact')
p.add_argument('--work',type=int,default=1);p.add_argument('--names',default='p999_case')
p.add_argument('--features',default='');p.add_argument('--drift');p.add_argument('--fail',action='store_true')
a=p.parse_args();destination=pathlib.Path(a.output)
# Outputs are clearly synthetic and confined by the parent test specification.
destination.write_text(json.dumps({'synthetic':True,'passed':not a.fail,'work':a.work,'profile':a.profile,'names':a.names.split(',')}))
if a.drift:
 path=pathlib.Path(a.drift);path.write_text(path.read_text()+'\nSYNTHETIC source drift')
for name in a.names.split(','):print('test '+name+' ... ok')
print('test result: ok. '+str(len(a.names.split(',')))+' passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;')
if a.fail:raise SystemExit(7)
'''

class WorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='workflow-synthetic-', dir='/tmp')
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name).resolve()
        self.repo = self.base/'repo';self.repo.mkdir()
        self.write(self.repo/'src/a.rs','// synthetic runtime identity\n')
        self.write(self.repo/'fixtures/case.wat',';; synthetic identity only, never encoded\n')
        self.write(self.repo/'harness/settings.json','{"synthetic":true}\n')
        self.write(self.repo/'produce.py',PRODUCER)
        # Copies permit source-identity invalidation tests without live edits.
        self.package_root=self.base/'modules/verification'
        self.package_root.parent.mkdir()
        if not MODULE_SOURCE.is_dir():
            self.fail('Root verification implementation is not available; staged tests must not be run yet')
        shutil.copytree(MODULE_SOURCE,self.package_root,ignore=shutil.ignore_patterns('__pycache__'))
        package_name='synthetic_verification_'+uuid.uuid4().hex
        init=self.package_root/'__init__.py'
        if not init.exists():self.write(init,'')
        package_spec=importlib.util.spec_from_file_location(package_name,init,submodule_search_locations=[str(self.package_root)])
        package=importlib.util.module_from_spec(package_spec);sys.modules[package_name]=package;package_spec.loader.exec_module(package)
        runner_spec=importlib.util.spec_from_file_location(package_name+'.runner',self.package_root/'runner.py')
        self.runner=importlib.util.module_from_spec(runner_spec);sys.modules[runner_spec.name]=self.runner;runner_spec.loader.exec_module(self.runner)
        self.addCleanup(lambda:[sys.modules.pop(name,None) for name in list(sys.modules) if name==package_name or name.startswith(package_name+'.')])
        self.workflow=self.runner.Workflow(self.repo,self.base/'store')
        self.spec=self.specification()
        self.spec_path=self.repo/'workflow.json';self.save()

    def write(self,path,text):
        path=Path(path);self.assertTrue(path.resolve(strict=False).is_relative_to(self.base),'synthetic write escaped fresh root')
        path.parent.mkdir(parents=True,exist_ok=True);path.write_text(text);return path

    def save(self):self.write(self.spec_path,json.dumps(self.spec))

    def specification(self):
        result={'schema_version':1,'task':'P999','synthetic':True,'identities':{
            'runtime':{'paths':['src/*.rs'],'settings':{}},
            'fixture':{'paths':['fixtures/*.wat'],'settings':{'profile':'compact'}},
            'harness':{'paths':['harness/*.json'],'settings':{'mode':'compact'}}},'stages':{},
            'completion':{'outcome':'contract_only','required_phases':['freeze','verify','measure'],'limitations':['Synthetic unit test only']}}
        for phase,uses in [('freeze',['runtime']),('verify',['runtime','fixture']),('measure',['runtime','harness'])]:
            result['stages'][phase]={'uses':uses,'dependencies':[],'commands':[{'name':'produce','argv':['{python}','{repo}/produce.py','{stage}/result.json']}],
                'outputs':{'result':'{stage}/result.json'},'checks':[{'artifact':'result','pointer':'/synthetic','op':'equal','value':True},{'artifact':'result','pointer':'/passed','op':'equal','value':True},{'artifact':'result','pointer':'/work','op':'positive'},{'artifact':'result','pointer':'/profile','op':'equal','value':'compact'}],
                'tests':[{'command':'produce','required_names':['p999_case'],'minimum_passed':1,'features':[]}]}
        return result

    def complete(self):
        return {phase:self.workflow.run(self.spec_path,phase) for phase in ('freeze','verify','measure')}

    def no_commands(self):
        # Finalization and successful cache reuse must never launch subprocesses.
        patcher=mock.patch.multiple('subprocess',run=mock.Mock(side_effect=AssertionError('Unexpected subprocess after successful phase')),Popen=mock.Mock(side_effect=AssertionError('Unexpected subprocess after successful phase')))
        return patcher

    def attempts(self):
        # Use retained file identities rather than a hardcoded store layout.
        return {p.relative_to(self.base).as_posix():p.read_bytes() for p in self.base.rglob('*') if p.is_file() and p.name not in {'index.json','cache.json','workflow-index.json','latest.json'} and 'modules' not in p.relative_to(self.base).parts and 'repo' not in p.relative_to(self.base).parts and p.name!='workflow.json'}

    def test_end_to_end_finalization_and_cache_reuse_launch_no_commands(self):
        self.complete()
        with self.no_commands():
            for phase in ('freeze','verify','measure'):self.workflow.run(self.spec_path,phase)
            output=self.base/'publication';final=self.workflow.finalize(self.spec_path,output)
        self.assertIsInstance(final,dict);self.assertIn('evidence',final);self.assertIn('completion_record',final)
        for key in ('evidence','completion_record'):
            path=Path(final[key]);self.assertTrue(path.resolve().is_relative_to(self.base));self.assertTrue(path.is_file())

    def test_packaging_only_change_does_not_replay_completed_phases(self):
        self.complete();self.spec['completion']['limitations'].append('Different packaging text');self.save()
        # Bookkeeper-only source change is also confined to the private copy.
        candidates=[self.package_root/name for name in ('finalize.py','publisher.py','bookkeeper.py') if (self.package_root/name).exists()]
        if candidates:self.write(candidates[0],candidates[0].read_text()+'\n# synthetic packaging-only source change\n')
        with self.no_commands():
            for phase in ('freeze','verify','measure'):self.workflow.run(self.spec_path,phase)
            result=self.workflow.finalize(self.spec_path,self.base/'repackaged')
        self.assertIn('completion_record',result)

    def test_identity_changes_only_invalidate_declared_consumers(self):
        self.complete()
        for identity,path,consumer,nonconsumer in [('fixture','fixtures/case.wat','verify','measure'),('harness','harness/settings.json','measure','verify')]:
            self.write(self.repo/path,(self.repo/path).read_text()+'\nsynthetic changed identity\n')
            with self.no_commands():self.workflow.run(self.spec_path,nonconsumer)
            with self.assertRaises(Exception):
                with self.no_commands():self.workflow.run(self.spec_path,consumer)
            self.workflow.run(self.spec_path,consumer)
        self.spec['identities']['harness']['settings']['mode']='synthetic_setting_change';self.save()
        with self.no_commands():self.workflow.run(self.spec_path,'verify')
        with self.assertRaises(Exception):
            with self.no_commands():self.workflow.run(self.spec_path,'measure')
        self.workflow.run(self.spec_path,'measure')
        self.write(self.repo/'src/a.rs','// changed synthetic runtime\n')
        for phase in ('freeze','verify','measure'):
            with self.assertRaises(Exception):
                with self.no_commands():self.workflow.run(self.spec_path,phase)

    def test_source_drift_fails_and_never_creates_reusable_success(self):
        stage=self.spec['stages']['verify'];stage['commands'][0]['argv']+=['--drift','{repo}/src/a.rs'];self.save()
        with self.assertRaises(Exception):self.workflow.run(self.spec_path,'verify')
        old=self.attempts()
        with self.assertRaises(Exception):self.workflow.run(self.spec_path,'verify')
        for path,data in old.items():self.assertEqual((self.base/path).read_bytes(),data,'Failed attempt bytes changed')
        self.assertGreater(len(self.attempts()),len(old),'Failed stage must create a fresh attempt on retry')

    def test_missing_named_test_zero_work_and_wrong_profile_fail(self):
        for extra in [['--names','wrong_case'],['--work','0'],['--profile','decoded']]:
            self.spec['stages']['verify']['commands'][0]['argv']=['{python}','{repo}/produce.py','{stage}/result.json',*extra];self.save()
            with self.assertRaises(Exception):self.workflow.run(self.spec_path,'verify')

    def test_escape_and_bad_output_templates_prevent_all_commands(self):
        for output in ['{repo}/forbidden.json','{stage}/../escape.json','{unknown}/bad.json']:
            self.spec['stages']['verify']['outputs']['result']=output;self.save()
            with mock.patch('subprocess.run') as run, mock.patch('subprocess.Popen') as popen:
                with self.assertRaises(Exception):self.workflow.plan(self.spec_path)
                run.assert_not_called();popen.assert_not_called()
            self.assertFalse((self.repo/'forbidden.json').exists())

    def test_combined_feature_cases_use_one_configured_invocation(self):
        stage=self.spec['stages']['verify'];names=['p999_alpha','p999_beta','p999_gamma']
        stage['commands'][0]['argv']+=['--names',','.join(names),'--features','compact-executor,execution-reference']
        stage['tests']=[{'command':'produce','required_names':names,'minimum_passed':3,'features':['compact-executor','execution-reference']}];self.save()
        import subprocess
        real=subprocess.run
        with mock.patch('subprocess.run',wraps=real) as invocation:
            self.workflow.run(self.spec_path,'verify')
        self.assertEqual(invocation.call_count,1,'Combined cases per feature set must execute once')

    def test_failed_stage_retry_keeps_prior_attempt_and_uses_fresh_success(self):
        self.spec['stages']['verify']['commands'][0]['argv'].append('--fail');self.save()
        with self.assertRaises(Exception):self.workflow.run(self.spec_path,'verify')
        old=self.attempts();self.spec['stages']['verify']['commands'][0]['argv'].remove('--fail');self.save()
        self.workflow.run(self.spec_path,'verify')
        for path,data in old.items():self.assertEqual((self.base/path).read_bytes(),data,'Retry overwrote failed attempt')
        with self.no_commands():self.workflow.run(self.spec_path,'verify')

    def test_interrupted_command_never_becomes_success_cache(self):
        with mock.patch('subprocess.run',side_effect=KeyboardInterrupt('Synthetic interruption')):
            with self.assertRaises(BaseException):self.workflow.run(self.spec_path,'verify')
        old=self.attempts();self.workflow.run(self.spec_path,'verify')
        for path,data in old.items():self.assertEqual((self.base/path).read_bytes(),data,'Retry changed interrupted attempt')
        with self.no_commands():self.workflow.run(self.spec_path,'verify')

if __name__=='__main__':unittest.main()
