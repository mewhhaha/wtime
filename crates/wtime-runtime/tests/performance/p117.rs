#![cfg(all(feature = "compact-executor", feature = "execution-reference"))]
use wtime_runtime::*;
const EH: &[u8]=include_bytes!("../fixtures/performance/p117/eh.wasm");
const POLICIES:[ExecutionPolicy;3]=[ExecutionPolicy::Decoded,ExecutionPolicy::Reference,ExecutionPolicy::Compact];
fn load(policy:ExecutionPolicy,fuel:Option<u64>,name:&str)->(Store,Func,Memory,Tag){
    let mut c=Config{features:FeatureSet::all_phase3_core(),..Config::default()};c.limits.fuel=fuel;
    let e=Engine::with_execution_policy(c,policy).unwrap();let m=e.compile(EH).unwrap();let mut s=Store::new(&e);
    let i=Linker::new(&e).instantiate(&mut s,&m).unwrap();
    let External::Func(f)=i.export(&s,name).unwrap() else{panic!()};
    let External::Memory(mem)=i.export(&s,"memory").unwrap() else{panic!()};
    let External::Tag(tag)=i.export(&s,"failure").unwrap() else{panic!()};(s,f,mem,tag)
}
fn observed(s:&mut Store,result:Result<RunOutcome>,tag:Tag)->(String,Vec<Value>){
    match result{
        Ok(RunOutcome::Returned(v))=>("returned".into(),v),
        Ok(RunOutcome::Exception(ex))=>{
            assert_eq!(s.exception_tag(ex).unwrap(),tag);let payload=s.exception_payload(ex).unwrap().to_vec();
            s.release_published(&[Value::Ref(RefValue::Exn(ex))]);s.collect().unwrap();
            assert!(s.exception_tag(ex).is_err());("exception".into(),payload)
        },
        Err(e)=>(format!("{:?}:{}",e.kind,e.message),vec![]),
        other=>panic!("unexpected {other:?}"),
    }
}
#[test]
fn p117_caught_transfer_preserves_all_fuel_prefixes_and_full_memory(){
    for fuel in 0..=14{
        let mut rows=vec![];
        for policy in POLICIES{
            let(mut s,f,m,tag)=load(policy,Some(fuel),"caught");let result=s.invoke(f,&[Value::I32(9)]);
            let result=observed(&mut s,result,tag);
            // Catch depth zero targets the function label and returns at the
            // throw; its normal terminal end is never executed.
            assert_eq!(result,if fuel<12{("ResourceLimit:fuel exhausted".into(),vec![])}else{("returned".into(),vec![Value::I32(10)])});
            let mut expected=vec![0;65536];if fuel>=8{expected[..4].copy_from_slice(&10i32.to_le_bytes());}
            let image=s.read_memory(m,0,65536).unwrap();assert_eq!(image,expected);
            if policy==ExecutionPolicy::Compact{
                let(steps,exits,last)=s.compact_prefix_counts();assert_eq!(steps,fuel.min(4)+u64::from(fuel>=11));
                assert_eq!(exits,[0,u64::from(fuel>=5)+u64::from(fuel>=12),0,0]);
                assert_eq!(last,if fuel>=12{Some((1,8))}else if fuel>=5{Some((4,0x1f))}else{None});
            }
            rows.push((result,image));
        }
        assert_eq!(rows[0],rows[1]);assert_eq!(rows[0],rows[2]);
    }
}
#[test]
fn p117_unhandled_exception_keeps_exact_tag_payload_and_source_exit(){
    for fuel in 0..=6{
        for policy in POLICIES{
            let(mut s,f,m,tag)=load(policy,Some(fuel),"uncaught");let result=s.invoke(f,&[Value::I32(9)]);
            assert_eq!(observed(&mut s,result,tag),if fuel<4{("ResourceLimit:fuel exhausted".into(),vec![])}else{("exception".into(),vec![Value::I32(11)])});
            assert!(s.read_memory(m,0,65536).unwrap().iter().all(|&b|b==0));
            if policy==ExecutionPolicy::Compact{assert_eq!(s.compact_prefix_counts(),(fuel.min(3),[0,u64::from(fuel>=4),0,0],if fuel>=4{Some((3,8))}else{None}));}
        }
    }
}
#[test]
fn p117_nested_unwind_and_precancellation_preserve_canonical_frames(){
    for fuel in 0..=8{
        for policy in POLICIES{
            let(mut s,f,m,tag)=load(policy,Some(fuel),"nested_uncaught");let result=s.invoke(f,&[Value::I32(9)]);
            assert_eq!(observed(&mut s,result,tag),if fuel<6{("ResourceLimit:fuel exhausted".into(),vec![])}else{("exception".into(),vec![Value::I32(12)])});
            if policy==ExecutionPolicy::Compact{
                assert_eq!(s.compact_dispatches(),fuel.min(5));
                assert_eq!(s.compact_prefix_counts(),(u64::from(fuel>=5),[0,u64::from(fuel>=6),0,0],if fuel>=6{Some((1,8))}else{None}));
            }
            let before=s.compact_prefix_counts();s.cancellation_token().cancel();
            assert_eq!(s.invoke(f,&[Value::I32(9)]).unwrap_err().message,"execution cancelled");
            assert_eq!(s.compact_prefix_counts(),before);assert!(s.read_memory(m,0,65536).unwrap().iter().all(|&b|b==0));
            s.cancellation_token().reset();s.collect().unwrap();assert_eq!(s.gc_payload_bytes(),0);
        }
    }
}
