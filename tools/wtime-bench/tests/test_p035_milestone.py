"""Synthetic safety tests for the staged P035 checker; not executed during P034."""
import hashlib, json, tempfile, unittest, sys
sys.dont_write_bytecode = True
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import structural_check as checker

class P035StructuralCheckTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(); self.root=Path(self.tmp.name)
    def tearDown(self): self.tmp.cleanup()
    def artifact(self,name,data):
        p=(self.root/name).resolve()
        if not p.is_relative_to(self.root.resolve()):
            raise AssertionError('synthetic artifact escaped fresh temp root')
        p.parent.mkdir(parents=True,exist_ok=True)
        raw=data if isinstance(data,bytes) else (json.dumps(data,sort_keys=True)+'\n').encode()
        p.write_bytes(raw); return {'path':str(p),'sha256':hashlib.sha256(raw).hexdigest(),'bytes':len(raw)}
    def test_missing_or_stale_artifact_fails_closed(self):
        ref=self.artifact('a.json',{'ok':True}); Path(ref['path']).write_text('{}')
        with self.assertRaises(checker.CheckError): checker.verify_ref(ref,'stale fixture')
        with self.assertRaises(checker.CheckError): checker.verify_ref({'path':str(self.root/'missing'),'sha256':'0'*64},'missing fixture')
    def test_pending_p034_cannot_pass_milestone(self):
        tasks={k:{'status':'done','outcome':'complete'} for k in checker.FOUNDATIONS}
        tasks['P017']={'status':'done','outcome':'adopted','promoted':True}
        for task in checker.CANDIDATES: tasks[task]={'status':'done','outcome':'rejected','promoted':False}
        tasks['P034']={'status':'todo','outcome':'pending','promoted':False}
        data={'tasks':tasks}
        with self.assertRaises(checker.CheckError): checker.validate_tasks(data)
    def test_rejected_candidate_is_preserved_but_never_promoted(self):
        data={'tasks':{k:{'status':'done','outcome':'complete'} for k in checker.FOUNDATIONS}}
        data['tasks']['P017']={'status':'done','outcome':'adopted','promoted':True}
        for task in checker.CANDIDATES:
            data['tasks'][task]={'status':'done','outcome':'rejected','promoted':False}
        data['tasks']['P034']={'status':'done','outcome':'rejected','promoted':False}
        index={'tasks':[{'id':key,'status':row['status'],'outcome':row['outcome']}
                        for key,row in data['tasks'].items()]}
        data['task_index']=self.artifact('task-index.json',index)
        data['tasks']['P028']['promoted']=True
        with self.assertRaises(checker.CheckError): checker.validate_tasks(data)
        data['tasks']['P028']['promoted']=False
        self.assertEqual(checker.validate_tasks(data)['P028']['outcome'],'rejected')
    def test_closed_task_rows_reconcile_with_task_index(self):
        tasks={key:{'status':'done','outcome':'complete','promoted':False}
               for key in checker.FOUNDATIONS}
        tasks['P017']={'status':'done','outcome':'adopted','promoted':True}
        for task in checker.CANDIDATES:
            tasks[task]={'status':'done','outcome':'rejected','promoted':False}
        tasks['P034']={'status':'done','outcome':'inconclusive','promoted':False}
        index={'tasks':[{'id':key,'status':row['status'],'outcome':row['outcome']}
                        for key,row in tasks.items()]}
        data={'tasks':tasks,'task_index':self.artifact('tasks.json',index)}
        self.assertEqual(checker.validate_tasks(data)['P034']['outcome'],'inconclusive')
    def test_instrumented_latency_is_rejected(self):
        registry={'selections':{name:{'status':'ready','workloads':[f'{name}-{i}' for i in range(n)]} for name,n in checker.SELECTIONS.items()},'schema_version':1}
        data={'workload_registry':self.artifact('registry.json',registry),
              'measurements':[{'selection':'scalar','instrumented':True,'mode':'decoded','runs':[]}]}
        with self.assertRaises(checker.CheckError): checker.validate_portfolio(data)
    def test_p001_cannot_claim_current_only_rows(self):
        self.assertEqual(checker.LEGACY,{'call_overhead','integer_loop','branch_calls'})
        self.assertFalse({'memory_fill_64','representative_image_filter_small'} <= checker.LEGACY)
        with self.assertRaises(checker.CheckError): checker.validate_p001_ids(checker.LEGACY|{'memory_fill_64'})
    def test_native_report_needs_both_runs_and_minimum_pairs_per_group(self):
        runs=[]
        for run_id in (1,2):
            pairs=[]
            for index in range(30):
                pairs.append({'workload':'only-case','stage':'warm_call','pair_id':f'{run_id}-{index}',
                              'baseline':{'mode':'decoded','ns_per_operation':2.0},
                              'candidate':{'mode':'decoded','ns_per_operation':1.0}})
            runs.append({'run_id':run_id,'seed':100+run_id,'pairs':pairs})
        report={'schema_version':2,'passed':True,'source_stable':True,'runs':runs}
        groups,pair_map,seeds,analysis_groups=checker.report_pairs(report,'synthetic',{'only-case'})
        self.assertEqual(len(pair_map),60)
        self.assertEqual(groups,{(1,'only-case','warm_call'):30,(2,'only-case','warm_call'):30})
        self.assertEqual(seeds,{'1':101,'2':102})
        report['runs'][1]['pairs'].pop()
        with self.assertRaises(checker.CheckError):
            checker.report_pairs(report,'synthetic',{'only-case'})
    def test_native_zero_index_runs_map_to_one_index_analysis_order(self):
        runs=[]
        for run_id in (0,1):
            pairs=[{'workload':'only-case','stage':'run_after_setup','pair_id':f'{run_id}-{i}',
                    'baseline':{'mode':'decoded','ns_per_operation':2},
                    'candidate':{'mode':'decoded','ns_per_operation':1}} for i in range(30)]
            runs.append({'run_id':run_id,'seed':run_id+7,'pairs':pairs})
        groups,pairs,seeds,analysis_groups=checker.report_pairs(
            {'schema_version':2,'passed':True,'source_stable':True,'runs':runs},
            'representative',{'only-case'})
        self.assertEqual(set(groups),{(0,'only-case','run_after_setup'),(1,'only-case','run_after_setup')})
        self.assertEqual(set(analysis_groups),{(1,'only-case','run_after_setup'),(2,'only-case','run_after_setup')})
        self.assertEqual(seeds,{'0':7,'1':8})
    def test_native_report_rejects_instrumented_or_zero_timing(self):
        base={'schema_version':2,'passed':True,'source_stable':True,
              'runs':[{'run_id':n,'pairs':[{'workload':'x','stage':'warm_call','pair_id':str(n),
                    'baseline':{'mode':'decoded','ns_per_operation':2},
                    'candidate':{'mode':'decoded','ns_per_operation':1}}]} for n in (1,2)]}
        # One pair per group is insufficient even if its observed values look plausible.
        with self.assertRaises(checker.CheckError): checker.report_pairs(base,'synthetic',{'x'})
        for run in base['runs']:
            run['pairs'] *= 30
            for index,pair in enumerate(run['pairs']): pair['pair_id']=f"{run['run_id']}-{index}"
        base['runs'][0]['pairs'][0]['candidate']['ns_per_operation']=0
        with self.assertRaises(checker.CheckError): checker.report_pairs(base,'synthetic',{'x'})
    def test_analysis_must_match_every_report_group_and_pair_count(self):
        expected={(1,'x','warm_call'):30,(2,'x','warm_call'):30}
        analysis={'schema_version':1,'source_schema_version':2,'metadata':{},
                  'per_run':[{'run':run,'workload':'x','stage':'warm_call','pairs':30}
                             for run in (1,2)]}
        self.assertEqual(len(checker.validate_analysis(None,'synthetic',analysis,expected)),2)
        analysis['per_run'].append({'run':3,'workload':'x','stage':'warm_call','pairs':30})
        with self.assertRaises(checker.CheckError):
            checker.validate_analysis(None,'synthetic',analysis,expected)
    def test_profile_inventory_accepts_only_source_bound_explicit_unsupported_rows(self):
        ids={'scalar':['call_overhead','integer_loop','branch_calls']+[f's{i}' for i in range(82)],
             'memory':['memory_fill_scan']+[f'm{i}' for i in range(34)],
             'representative':[f'r{i}' for i in range(9)]}
        registry={'workloads':[],'selections':{}}
        for name,workloads in ids.items():
            registry['selections'][name]={'workloads':workloads}
            for wid in workloads:
                registry['workloads'].append({'id':wid,'source':{'sha256':'a'*64,'wasm_sha256':'b'*64}})
        supported={'scalar':set(checker.LEGACY),'memory':{'memory_fill_scan'},'representative':set(ids['representative'])}
        supported['scalar']=set(checker.LEGACY)
        supported['memory']={'memory_fill_scan'}
        rows=[]
        for selection,workloads in ids.items():
            cases=[]; counters=[]; cpu=[]
            for wid in workloads:
                if wid in supported[selection]:
                    artifact=f'{selection}/counters/{wid}.json'
                    ref=self.artifact(artifact,{'workload':wid})
                    counters.append({'workload':wid,'instrumented':True,'role':'diagnostic_profile',
                                     'binary_sha256':'c'*64,'fixture_sha256':'b'*64,'wat_sha256':'a'*64,
                                     'raw_observation':{'diagnostic':True,'error':None},
                                     'artifact':f'counters/{wid}.json','artifact_sha256':ref['sha256']})
                    cases.append({'workload':wid,'counter_status':'passed'})
                else:
                    cases.append({'workload':wid,'counter_status':'unsupported','reason':'not in declared route'})
                cpu.append({'workload':wid,'available':False,'attempted':False})
            report={'schema_version':1,'passed':True,'selection':selection,
                    'production_timing':False,'evidence_kind':'bounded_diagnostic_attribution',
                    'metadata':{'source_inputs':{'crates/wtime-runtime/src/lib.rs':'d'*64},
                                'artifacts':{'counters':{'binary_sha256':'c'*64}}},
                    'cases':cases,'counters':counters,'cpu_profiles':cpu,
                    'counts':{'selected':len(workloads),'actual_counter_cases':len(supported[selection]),
                              'explicit_unsupported_cases':len(workloads)-len(supported[selection])}}
            ref=self.artifact(f'{selection}/profile.json',report)
            rows.append({'selection':selection,'status':'passed','instrumented':True,
                         'role':'diagnostic_profile','source_map_sha256':'e'*64,'report':ref})
        runtime={'crates/wtime-runtime/src/lib.rs':'d'*64}
        checker.validate_profiles(rows,registry,runtime,'e'*64)
        profile_path=Path(next(row for row in rows if row['selection']=='scalar')['report']['path'])
        scalar=json.loads(profile_path.read_text())
        unsupported=next(row for row in scalar['cases'] if row['counter_status']=='unsupported')
        unsupported['reason']=''
        bad=self.artifact('scalar/profile-bad.json',scalar)
        bad_row=next(row for row in rows if row['selection']=='scalar')
        bad_row['report']=bad
        with self.assertRaises(checker.CheckError):
            checker.validate_profiles(rows,registry,runtime,'e'*64)
    def test_zero_or_missing_work_does_not_fill_coverage(self):
        registry={'selections':{name:{'status':'ready','workloads':[f'{name}-{i}' for i in range(n)]} for name,n in checker.SELECTIONS.items()}}
        data={'workload_registry':self.artifact('registry.json',registry),
              'measurements':[],'p001_legacy':{}}
        with self.assertRaises(checker.CheckError): checker.validate_portfolio(data)
    def test_source_chain_requires_p034_and_p033_runtime_equality(self):
        files={f'crates/wtime-runtime/src/file{i}.rs':f'{i:064x}' for i in range(checker.RUNTIME_COUNT)}
        # runtime_map accepts only correctly shaped hashes; this one demonstrates 20-file requirement.
        self.assertEqual(len(checker.runtime_map(files,'synthetic')),20)
        altered=dict(files); altered[next(iter(files))]='f'*64
        self.assertNotEqual(checker.runtime_map(files,'before'),checker.runtime_map(altered,'current'))

if __name__=='__main__': unittest.main()
