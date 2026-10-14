#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
use wtime_runtime::*;
const GC: &[u8] = include_bytes!("../fixtures/performance/p115/gc-active-local.wasm");
const POLICIES: [ExecutionPolicy;3] = [ExecutionPolicy::Decoded, ExecutionPolicy::Reference, ExecutionPolicy::Compact];
fn function(s: &Store, i: Instance, name: &str) -> Func {
    let External::Func(f) = i.export(s, name).unwrap() else { panic!() }; f
}
struct Loaded { store: Store, run: Func, value: Value, calls: Arc<AtomicUsize>, before: (u64,[u64;4],Option<(usize,u16)>) }
fn loaded(policy: ExecutionPolicy, fuel: Option<u64>, nested: bool) -> Loaded {
    let mut config = Config { features: FeatureSet::all_phase3_core(), ..Config::default() }; config.limits.fuel = fuel;
    let e = Engine::with_execution_policy(config, policy).unwrap(); let m = e.compile(GC).unwrap();
    let mut s = Store::new(&e); let mut linker = Linker::new(&e);
    let nested_func = if nested {
        let module = e.compile(include_bytes!("../fixtures/performance/p038/explicit-return.wasm")).unwrap();
        let i = linker.instantiate(&mut s, &module).unwrap(); Some(function(&s,i,"run"))
    } else { None };
    let baseline = Arc::new(Mutex::new((0u64,[0u64;4]))); let inside = baseline.clone();
    let calls = Arc::new(AtomicUsize::new(0)); let called = calls.clone();
    let host = s.create_host_func_with_caller(FuncType::new(vec![],vec![]), move |caller,args| {
        assert!(args.is_empty()); // The reference is never a host argument/root.
        if policy == ExecutionPolicy::Compact {
            let (steps,exits,_) = caller.store().compact_prefix_counts(); let before = *inside.lock().unwrap();
            assert_eq!(steps-before.0,7); assert_eq!(exits,before.1);
        }
        assert_eq!(caller.collect()?.live,1);
        if let Some(f) = nested_func {
            assert_eq!(caller.invoke(f,&[])?,RunOutcome::Returned(vec![Value::I32(42)]));
            assert_eq!(caller.collect()?.live,1);
        }
        called.fetch_add(1,Ordering::SeqCst); Ok(vec![])
    }).unwrap();
    linker.define("env","collect",External::Func(host)).unwrap(); let i=linker.instantiate(&mut s,&m).unwrap();
    let make=function(&s,i,"make"); let run=function(&s,i,"run");
    let RunOutcome::Returned(values)=s.invoke(make,&[Value::I32(73)]).unwrap() else { panic!() };
    let [value]=values.as_slice() else { panic!() }; let value=*value;
    s.release_published(&values); // No embedding RootToken or publication survives.
    let before=s.compact_prefix_counts(); *baseline.lock().unwrap()=(before.0,before.1);
    Loaded { store:s,run,value,calls,before }
}
#[test]
fn p116_sole_active_compact_local_survives_zero_argument_host_collection() {
    for policy in POLICIES {
        let mut c=loaded(policy,None,false);
        assert_eq!(c.store.invoke(c.run,&[c.value]).unwrap(),RunOutcome::Returned(vec![Value::I32(73)]));
        assert_eq!(c.calls.load(Ordering::SeqCst),1);
        if policy==ExecutionPolicy::Compact {
            let (steps,exits,last)=c.store.compact_prefix_counts(); assert_eq!(steps-c.before.0,8);
            assert_eq!(exits[0]-c.before.1[0],1); assert_eq!(last,Some((8,0xfb02)));
        }
        let stats=c.store.collect().unwrap(); assert_eq!((stats.live,stats.reclaimed),(0,1));
        assert_eq!(c.store.gc_payload_bytes(),0);
    }
}
#[test]
fn p116_nested_numeric_invoke_keeps_outer_reference_local_visible() {
    for policy in POLICIES {
        let mut c=loaded(policy,None,true);
        assert_eq!(c.store.invoke(c.run,&[c.value]).unwrap(),RunOutcome::Returned(vec![Value::I32(73)]));
        assert_eq!(c.calls.load(Ordering::SeqCst),1);
        assert_eq!(c.store.collect().unwrap().live,0); assert_eq!(c.store.gc_payload_bytes(),0);
        if policy==ExecutionPolicy::Compact { assert_eq!(c.store.compact_prefix_counts().0-c.before.0,8); }
    }
}
#[test]
fn p116_low_fuel_cancellation_foreign_and_stale_refs_release_roots() {
    for policy in POLICIES {
        for fuel in 3..=10 {
            let mut c=loaded(policy,Some(fuel),false); let outcome=c.store.invoke(c.run,&[c.value]);
            if fuel<10 { let e=outcome.unwrap_err(); assert_eq!((e.kind,e.message),(ErrorKind::ResourceLimit,"fuel exhausted".into())); }
            else { assert_eq!(outcome.unwrap(),RunOutcome::Returned(vec![Value::I32(73)])); }
            assert_eq!(c.calls.load(Ordering::SeqCst),usize::from(fuel>=7));
            if policy==ExecutionPolicy::Compact {
                let (steps,exits,last)=c.store.compact_prefix_counts(); assert_eq!(steps-c.before.0,fuel.min(8));
                assert_eq!(exits[0]-c.before.1[0],u64::from(fuel>=9));
                assert_eq!(last,if fuel>=9 { Some((8,0xfb02)) } else { c.before.2 });
            }
            let stats=c.store.collect().unwrap(); assert_eq!((stats.live,stats.reclaimed),(0,1));
        }
        let mut c=loaded(policy,None,false); let mut foreign=loaded(policy,None,false);
        assert_eq!(foreign.store.invoke(foreign.run,&[c.value]).unwrap_err().kind,ErrorKind::Host);
        assert_eq!(foreign.store.compact_prefix_counts(),foreign.before);
        c.store.cancellation_token().cancel(); assert_eq!(c.store.invoke(c.run,&[c.value]).unwrap_err().message,"execution cancelled");
        assert_eq!(c.store.compact_prefix_counts(),c.before); c.store.cancellation_token().reset();
        assert_eq!(c.store.collect().unwrap().reclaimed,1);
        assert_eq!(c.store.invoke(c.run,&[c.value]).unwrap_err().kind,ErrorKind::Host);
        assert_eq!(c.store.compact_prefix_counts(),c.before);
        assert_eq!(foreign.store.collect().unwrap().live,0);
    }
}
