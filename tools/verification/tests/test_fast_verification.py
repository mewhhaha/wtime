"""Fast-verification integrity tests: confined synthetic roots, no live evidence."""
import copy
import gzip
import importlib.util
import json
import os
from pathlib import Path
import sys
import tracemalloc
import unittest
from unittest import mock
sys.dont_write_bytecode=True
ROOT_TESTS=Path('/workspace/wtime/tools/verification/tests')
spec=importlib.util.spec_from_file_location('fast_runner_fixture',ROOT_TESTS/'test_runner.py')
fixtures=importlib.util.module_from_spec(spec);spec.loader.exec_module(fixtures)

class FastVerificationTests(unittest.TestCase):
    setUp=fixtures.WorkflowTests.setUp
    write=fixtures.WorkflowTests.write
    save=fixtures.WorkflowTests.save
    no_commands=fixtures.WorkflowTests.no_commands
    def specification(self):
        value=fixtures.WorkflowTests.specification(self);value['synthetic']=True
        value['completion']['required_phases']=['verify'];return value
    def load(self,name):
        fullname=self.runner.__package__+'.'+name
        if fullname in sys.modules:return sys.modules[fullname]
        spec=importlib.util.spec_from_file_location(fullname,self.package_root/(name+'.py'))
        module=importlib.util.module_from_spec(spec);sys.modules[fullname]=module;spec.loader.exec_module(module);return module
    def object(self,payload=b'synthetic small object\n'):
        artifacts=self.load('artifacts');store=artifacts.ArtifactStore(self.base/'cas')
        source=self.repo/'object.bin';self.assertTrue(source.resolve(strict=False).is_relative_to(self.base));source.write_bytes(payload)
        reference=store.add(source);return store,reference,store.root/reference['path']

    def test_aliases_deduplicate_raw_pass_without_accepting_bad_metadata(self):
        context=self.load('context');store,reference,path=self.object()
        with context.operation() as operation:
            self.assertIs(context.current(),operation)
            store.verify(reference);alias=copy.deepcopy(reference);alias['mode']='0o600';store.verify(alias);store.verify(copy.deepcopy(reference))
            self.assertEqual(operation.summary()['artifact_verifications'],1)
            self.assertEqual(operation.summary()['artifact_cache_hits'],2)
            self.assertEqual(operation.summary()['artifact_raw_bytes_verified'],reference['bytes'])
            for key,value in [('bytes',reference['bytes']+1),('stored_bytes',reference['stored_bytes']+1),('stored_sha256','0'*64),('sha256','1'*64),('mode','malformed')]:
                altered=copy.deepcopy(reference);altered[key]=value
                with self.assertRaises(Exception):store.verify(altered)
        self.assertIsNone(context.current())

    def test_same_operation_chmod_content_and_inode_replacement_rejected(self):
        context=self.load('context')
        for damage in ('chmod','content','replace'):
            with self.subTest(damage=damage):
                # Each subcase receives a separately named CAS root and fresh source.
                artifacts=self.load('artifacts');store=artifacts.ArtifactStore(self.base/('cas-'+damage));source=self.write(self.repo/(damage+'.txt'),'synthetic original bytes\n');reference=store.add(source);path=store.root/reference['path'];stamp=path.stat()
                with self.assertRaises(Exception):
                    with context.operation():
                        store.verify(reference)
                        if damage=='chmod':os.chmod(path,0o644)
                        elif damage=='content':
                            os.chmod(path,0o644);path.write_bytes(b'X'*reference['bytes']);os.chmod(path,0o444);os.utime(path,ns=(stamp.st_atime_ns,stamp.st_mtime_ns))
                        else:
                            replacement=path.with_name(path.name+'.synthetic-replacement');self.assertTrue(replacement.resolve(strict=False).is_relative_to(self.base));replacement.write_bytes(path.read_bytes());os.chmod(replacement,0o444);os.utime(replacement,ns=(stamp.st_atime_ns,stamp.st_mtime_ns));os.replace(replacement,path)
                        store.verify(reference)

    def test_verify_streams_large_raw_and_gzip_without_full_payload_reads(self):
        context=self.load('context');artifacts=self.load('artifacts');size=8*context.CHUNK+333
        for encoding in ('raw','gzip'):
            with self.subTest(encoding=encoding):
                data=(bytes(range(251))*((size//251)+1))[:size]
                if encoding=='gzip':data=b'\x7fELF'+data[4:]
                store=artifacts.ArtifactStore(self.base/('large-'+encoding));source=self.repo/(encoding+'.bin');self.assertTrue(source.resolve(strict=False).is_relative_to(self.base));source.write_bytes(data);reference=store.add(source)
                self.assertEqual(reference['encoding'],encoding)
                calls=[];original=gzip.GzipFile.read
                def bounded(stream,size=-1):
                    self.assertGreater(size,0);self.assertLessEqual(size,context.CHUNK);calls.append(size);return original(stream,size)
                tracemalloc.start()
                try:
                    with mock.patch.object(store,'read',side_effect=AssertionError('verify must not collect raw bytes')),mock.patch.object(Path,'read_bytes',side_effect=AssertionError('verify must stream')),mock.patch.object(gzip.GzipFile,'read',autospec=True,side_effect=bounded),context.operation() as operation:
                        store.verify(reference);summary=operation.summary()
                    _,peak=tracemalloc.get_traced_memory()
                finally:tracemalloc.stop()
                self.assertEqual(summary['artifact_raw_bytes_verified'],len(data));self.assertEqual(summary['artifact_stored_bytes_read'],reference['stored_bytes']);self.assertEqual(summary['artifact_verifications'],1)
                self.assertLess(peak,len(data),'verification retained a complete large payload')
                if encoding=='gzip':self.assertGreater(len(calls),1)

    def test_operation_reset_rehashes_and_detects_later_tamper(self):
        context=self.load('context');store,reference,path=self.object()
        with context.operation() as first:store.verify(reference)
        with context.operation() as second:store.verify(reference)
        self.assertIsNot(first,second);self.assertEqual(first.summary()['artifact_verifications'],1);self.assertEqual(second.summary()['artifact_verifications'],1)
        stamp=path.stat();os.chmod(path,0o644);path.write_bytes(b'X'*reference['bytes']);os.chmod(path,0o444);os.utime(path,ns=(stamp.st_atime_ns,stamp.st_mtime_ns))
        with context.operation() as third:
            with self.assertRaises(Exception):store.verify(reference)
            self.assertGreater(third.summary()['artifact_raw_bytes_verified'],0);self.assertEqual(third.summary().get('artifact_cache_hits',0),0)

    def test_guard_detects_new_identity_glob_member(self):
        context=self.load('context');contracts=self.load('contracts')
        with self.assertRaisesRegex(Exception,'membership'):
            with context.operation():
                contracts.identity(self.repo,{'paths':['src/*.rs'],'settings':{}})
                self.write(self.repo/'src/new.rs','// synthetic added glob member\n')

    def test_nested_diamond_phase_inputs_and_receipts_computed_once(self):
        context=self.load('context')
        freeze=copy.deepcopy(self.spec['stages']['freeze']);verify=copy.deepcopy(self.spec['stages']['verify']);measure=copy.deepcopy(self.spec['stages']['measure']);analyze=copy.deepcopy(measure)
        verify['dependencies']=['freeze'];measure['dependencies']=['freeze'];analyze['dependencies']=['verify','measure']
        self.spec['stages']={'freeze':freeze,'verify':verify,'measure':measure,'analyze':analyze};self.save()
        self.workflow.run_phases(self.spec_path,['freeze','verify','measure','analyze'])
        with self.no_commands(),context.operation() as operation:
            self.workflow._inputs(self.spec,'analyze');self.workflow._inputs(self.spec,'analyze')
            for phase in ('freeze','verify','measure'):self.workflow._receipt('P999',phase)
            summary=operation.summary()
        self.assertEqual(summary['phase_inputs_computations'],4);self.assertEqual(summary['receipts_computations'],3)
        self.assertGreater(summary['phase_inputs_cache_hits'],0);self.assertGreater(summary['receipts_cache_hits'],0)
        with self.no_commands(),context.operation() as second:self.workflow._inputs(self.spec,'analyze')
        self.assertEqual(second.summary()['phase_inputs_computations'],4)

    def test_development_lane_cannot_finalize_or_supply_full_receipts(self):
        self.spec['development_lane']=True;self.save()
        development=self.runner.Workflow(self.repo,self.base/'development-store',lane='development')
        receipt=development.run(self.spec_path,'verify');self.assertEqual(receipt['lane'],'development');self.assertIs(receipt['completion_eligible'],False)
        with self.no_commands():
            with self.assertRaisesRegex(Exception,'development'):development.finalize(self.spec_path,self.base/'dev-final')
            with self.assertRaisesRegex(Exception,'development'):self.workflow.run(self.spec_path,'verify')
        self.spec['development_lane']=False;self.save()
        full=self.runner.Workflow(self.repo,development.root,lane='full')
        with self.no_commands():
            with self.assertRaises(Exception):full.finalize(self.spec_path,self.base/'foreign-dev-final')
        # Publisher must reject even a syntactically completion-shaped dev proof.
        publisher=self.load('publish');artifacts=self.load('artifacts');workspace=self.base/'publisher';performance=workspace/'tasks/performance';repo=workspace/'wtime'
        self.write(performance/'TASKS.json',json.dumps({'tasks':[{'id':'P999','path':'P999.md','status':'todo','outcome':'pending','dependencies':[]}]}));self.write(performance/'INDEX.md','All 1 tasks are planned.\n');self.write(performance/'P999.md','Status: **todo**\nOutcome: **pending**\n\nNo commands in this planning file have been executed as implementation evidence.\n');self.write(performance/'check_plan.py',"STATUSES={'todo','done'}\nOUTCOMES={'pending','contract_only'}\n")
        proof=self.write(workspace/'proof.json',json.dumps({'task':'P999','status':'passed','passed':True,'outcome':'contract_only','lane':'development','completion_eligible':False}));reference=artifacts.normal(proof);record=self.write(workspace/'completion.json',json.dumps({'task':'P999','status':'done','outcome':'contract_only','evidence':[reference],'artifact_hashes':{str(proof):reference['sha256']}}));status=repo/'target/performance/TASK-status';status.mkdir(parents=True)
        original=(performance/'TASKS.json').read_bytes()
        with mock.patch('subprocess.run') as run:self.assertEqual(publisher.execute(performance,record,status/'attempt'),1);run.assert_not_called()
        attempt=json.loads((status/'attempt/publication-attempt.json').read_text());self.assertIn('development',attempt['error']);self.assertEqual((performance/'TASKS.json').read_bytes(),original)

    def test_directory_and_trailing_recursive_patterns_capture_files_and_guard_additions(self):
        context=self.load('context');contracts=self.load('contracts')
        self.write(self.repo/'tree/first.txt','synthetic first\n')
        self.write(self.repo/'tree/nested/second.txt','synthetic second\n')
        expected={'tree/first.txt','tree/nested/second.txt'}
        for pattern in ('tree','tree/**'):
            with self.subTest(pattern=pattern):
                with context.operation() as operation:
                    declaration={'paths':[pattern],'settings':{}}
                    first=contracts.identity(self.repo,declaration)
                    second=contracts.identity(self.repo,declaration)
                    self.assertEqual(set(first['files']),expected)
                    self.assertEqual(first,second)
                    self.assertEqual(operation.summary()['hash_passes'],2)
                    self.assertEqual(operation.summary()['hash_cache_hits'],2)
                added=self.repo/'tree/nested/added.txt'
                with self.assertRaisesRegex(Exception,'membership'):
                    with context.operation():
                        contracts.identity(self.repo,declaration)
                        self.write(added,'synthetic new member\n')
                added.unlink()

    def test_identity_symlink_retarget_detected_with_unchanged_targets(self):
        context=self.load('context');contracts=self.load('contracts')
        first=self.write(self.repo/'targets/first.txt','same bytes\n')
        second=self.write(self.repo/'targets/second.txt','same bytes\n')
        stamp=first.stat();os.utime(second,ns=(stamp.st_atime_ns,stamp.st_mtime_ns))
        link=self.repo/'identity.txt';link.symlink_to(first.relative_to(self.repo))
        original={p:p.stat() for p in (first,second)}
        with self.assertRaisesRegex(Exception,'membership'):
            with context.operation():
                identity=contracts.identity(self.repo,{'paths':['identity.txt'],'settings':{}})
                self.assertEqual(set(identity['files']),{'identity.txt'})
                link.unlink();link.symlink_to(second.relative_to(self.repo))
                for path,before in original.items():
                    after=path.stat()
                    self.assertEqual((after.st_ino,after.st_size,after.st_mtime_ns),(before.st_ino,before.st_size,before.st_mtime_ns))
                    self.assertEqual(path.read_text(),'same bytes\n')

if __name__=='__main__':unittest.main()
