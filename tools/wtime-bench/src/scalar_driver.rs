//! Explicit host/table/correctness driver for P008 scalar fixtures.
//!
//! This leaf is wired into the harness by an integrator; keeping it separate
//! leaves the stable protocol and its original single-case path unchanged.

mod current {
    use std::{hint::black_box, time::Instant};

    use wtime_runtime::{
        Config, Engine, ErrorKind, External, Func, FuncType, Instance, Linker,
        RunOutcome, Store, TrapCode, Value,
    };
    use wtime_types::{FeatureSet, ValType};
    #[cfg(not(feature = "legacy-api"))]
    use wtime_runtime::ExecutionPolicy;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Driver {
        Plain,
        HostTick,
        TableA,
        TableB,
        ProbeNull,
        ProbeMismatch,
        DepthProbe,
        TailProbe,
    }

    impl Driver {
        fn parse(value: &str) -> Option<Self> {
            Some(match value {
                "plain" => Self::Plain,
                "host_tick" => Self::HostTick,
                "table_a" => Self::TableA,
                "table_b" => Self::TableB,
                "probe_null" => Self::ProbeNull,
                "probe_mismatch" => Self::ProbeMismatch,
                "depth_probe" => Self::DepthProbe,
                "tail_probe" => Self::TailProbe,
                _ => return None,
            })
        }

        fn is_probe(self) -> bool {
            matches!(
                self,
                Self::ProbeNull | Self::ProbeMismatch | Self::DepthProbe | Self::TailProbe
            )
        }

        fn table_target(self) -> Option<i32> {
            match self {
                Self::TableA => Some(0),
                Self::TableB => Some(1),
                _ => None,
            }
        }
    }

    fn quote(value: &str) -> String {
        let mut out = String::from("\"");
        for ch in value.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    fn configured_engine(mode: &str, profile: &str, bounded_depth: bool) -> Result<Engine, String> {
        #[cfg(not(feature = "legacy-api"))]
        let policy = match mode {
            "decoded" => ExecutionPolicy::Decoded,
            "portable" => ExecutionPolicy::Portable,
            "reference" => ExecutionPolicy::Reference,
            "compact" => ExecutionPolicy::Compact,
            "register" => ExecutionPolicy::Register,
            "experimental" => ExecutionPolicy::Experimental,
            "native" => ExecutionPolicy::Native,
            _ => return Err(format!("unsupported execution mode {mode}")),
        };
        let features = match profile {
            "mvp" => FeatureSet::mvp(),
            "mvp-saturating" => { let mut features=FeatureSet::mvp(); features.saturating_float_to_int=true; features },
            "simd" | "simd-relaxed" => { let mut features=FeatureSet::mvp(); features.simd=true; features.saturating_float_to_int=true; features.relaxed_simd=profile=="simd-relaxed"; features },
            "stable-core" => FeatureSet::stable_core(),
            "all-phase3-core" => FeatureSet::all_phase3_core(),
            _ => return Err(format!("unsupported feature profile {profile}")),
        };
        let mut config = Config {
            features,
            ..Config::default()
        };
        if bounded_depth {
            config.limits.max_call_depth = 32;
        }
        #[cfg(feature = "legacy-api")]
        {
            if mode != "decoded" { return Err(format!("unavailable P001 mode {mode}")); }
            Engine::new(config).map_err(|error| error.to_string())
        }
        #[cfg(not(feature = "legacy-api"))]
        Engine::with_execution_policy(config, policy).map_err(|error| error.to_string())
    }

    fn returned_i32(store: &mut Store, func: Func, args: &[Value]) -> i32 {
        match store.invoke(func, black_box(args)).unwrap() {
            RunOutcome::Returned(values) => match values.as_slice() {
                [Value::I32(value)] => *value,
                other => panic!("expected one i32 result, got {other:?}"),
            },
            other => panic!("unexpected guest outcome {other:?}"),
        }
    }

    fn no_result(store: &mut Store, func: Func, args: &[Value]) {
        match store.invoke(func, black_box(args)).unwrap() {
            RunOutcome::Returned(values) if values.is_empty() => {}
            other => panic!("expected no results, got {other:?}"),
        }
    }

    fn export(store: &Store, instance: Instance, name: &str) -> Func {
        match instance.export(store, name).unwrap() {
            External::Func(func) => func,
            other => panic!("{name} must be a function, got {other:?}"),
        }
    }

    fn linker(engine: &Engine, store: &mut Store, driver: Driver) -> Linker {
        let mut linker = Linker::new(engine);
        if driver == Driver::HostTick {
            let callback = store
                .create_host_func(
                    FuncType::new(vec![ValType::I32], vec![ValType::I32]),
                    |args| match args {
                        [Value::I32(value)] => Ok(vec![Value::I32(value.wrapping_add(1))]),
                        other => Err(wtime_runtime::Error::new(
                            ErrorKind::Host,
                            format!("host.tick expects one i32, got {other:?}"),
                        )),
                    },
                )
                .unwrap();
            linker
                .define("host", "tick", External::Func(callback))
                .unwrap();
        }
        linker
    }

    fn instantiate(
        engine: &Engine,
        module: &wtime_runtime::Module,
        driver: Driver,
    ) -> (Store, Instance) {
        let mut store = Store::new(engine);
        let linker = linker(engine, &mut store, driver);
        let instance = linker.instantiate(&mut store, module).unwrap();
        if let Some(target) = driver.table_target() {
            let set_target = export(&store, instance, "set_target");
            no_result(&mut store, set_target, &[Value::I32(target)]);
        }
        (store, instance)
    }

    fn check_call(
        store: &mut Store,
        instance: Instance,
        driver: Driver,
        input: i32,
        expected: i32,
    ) -> i32 {
        let func = export(store, instance, "run");
        let result = returned_i32(store, func, &[Value::I32(input)]);
        assert_eq!(result, expected);
        if driver == Driver::TableA || driver == Driver::TableB {
            // The setup is outside timing and each timed invocation starts with
            // a fresh accumulator, so repeated calls must preserve the oracle.
            let target = driver.table_target().unwrap();
            let set_target = export(store, instance, "set_target");
            no_result(store, set_target, &[Value::I32(target)]);
        }
        result
    }

    fn invoke_timed(store: &mut Store, func: Func, input: i32) -> i32 {
        returned_i32(store, func, &[Value::I32(input)])
    }

    fn measured(batch: usize, mut call: impl FnMut() -> i32, expected: i32) -> (f64, i32) {
        let start = Instant::now();
        let mut result = 0;
        for _ in 0..batch {
            result = black_box(call());
        }
        let elapsed = start.elapsed().as_nanos() as f64 / batch as f64;
        assert_eq!(result, expected, "oracle mismatch");
        (elapsed.max(1.0), result)
    }

    fn json_timing(
        case: &str,
        stage: &str,
        input: i32,
        expected: i32,
        result: i32,
        batch: usize,
        elapsed: f64,
        mode: &str,
        profile: &str,
        driver: Driver,
    ) {
        let driver = match driver {
            Driver::Plain => "plain",
            Driver::HostTick => "host_tick",
            Driver::TableA => "table_a",
            Driver::TableB => "table_b",
            Driver::ProbeNull => "probe_null",
            Driver::ProbeMismatch => "probe_mismatch",
            Driver::DepthProbe => "depth_probe",
            Driver::TailProbe => unreachable!(),
        };
        println!(
            "{{\"schema_version\":2,\"case\":{},\"stage\":{},\"input\":{},\"expected\":{},\"result\":{},\"batch\":{},\"ns_per_operation\":{:.3},\"mode\":{},\"profile\":{},\"driver\":{},\"diagnostic\":false}}",
            quote(case),
            quote(stage),
            quote(&format!("i32:{input}")),
            quote(&format!("i32:{expected}")),
            quote(&format!("i32:{result}")),
            batch,
            elapsed.max(1.0),
            quote(mode),
            quote(profile),
            quote(driver),
        );
    }

    fn json_probe(
        case: &str,
        mode: &str,
        profile: &str,
        driver: Driver,
        kind: &str,
        message: &str,
    ) {
        let driver = match driver {
            Driver::ProbeNull => "probe_null",
            Driver::ProbeMismatch => "probe_mismatch",
            Driver::DepthProbe => "depth_probe",
            Driver::TailProbe => unreachable!(),
            _ => unreachable!(),
        };
        println!(
            "{{\"schema_version\":2,\"case\":{},\"driver\":{},\"diagnostic\":true,\"status\":\"trap_verified\",\"trap_kind\":{},\"message\":{},\"mode\":{},\"profile\":{}}}",
            quote(case),
            quote(driver),
            quote(kind),
            quote(message),
            quote(mode),
            quote(profile),
        );
    }

    fn probe(
        engine: &Engine,
        module: &wtime_runtime::Module,
        case: &str,
        mode: &str,
        profile: &str,
        driver: Driver,
        input: i32,
        expected: i32,
    ) {
        let (mut store, instance) = instantiate(engine, module, driver);
        if driver == Driver::TailProbe {
            assert_eq!(input, 10000, "tail probe uses the specified 10000 input");
            let func = export(&store, instance, "run");
            let result = returned_i32(&mut store, func, &[Value::I32(input)]);
            assert_eq!(result, expected, "tail recursion oracle mismatch");
            println!(
                "{{\"schema_version\":2,\"case\":{},\"driver\":\"tail_probe\",\"diagnostic\":true,\"status\":\"returned\",\"input\":{},\"expected\":{},\"result\":{},\"mode\":{},\"profile\":{}}}",
                quote(case),
                quote(&format!("i32:{input}")),
                quote(&format!("i32:{expected}")),
                quote(&format!("i32:{result}")),
                quote(mode),
                quote(profile),
            );
            return;
        }
        let outcome = match driver {
            Driver::ProbeNull => {
                let func = export(&store, instance, "probe_null");
                store.invoke(func, &[])
            }
            Driver::ProbeMismatch => {
                let func = export(&store, instance, "probe_mismatch");
                store.invoke(func, &[])
            }
            Driver::DepthProbe => {
                assert!(
                    matches!(input, 1024 | 10000),
                    "depth probe inputs are 1024 or 10000"
                );
                let func = export(&store, instance, "run");
                store.invoke(func, &[Value::I32(input)])
            }
            _ => unreachable!(),
        };
        match outcome {
            Err(error) => {
                let valid = match driver {
                    Driver::ProbeNull => {
                        error.kind == ErrorKind::Trap
                            && error.trap_code == Some(TrapCode::NullReference)
                    }
                    Driver::ProbeMismatch => {
                        error.kind == ErrorKind::Trap
                            && error.trap_code == Some(TrapCode::IndirectCallTypeMismatch)
                    }
                    Driver::DepthProbe => {
                        error.kind == ErrorKind::ResourceLimit
                            && error.message.to_ascii_lowercase().contains("call depth")
                    }
                    _ => false,
                };
                assert!(valid, "unexpected probe failure: {error:?}");
                let kind = if error.kind == ErrorKind::ResourceLimit {
                    format!("{:?}", error.kind)
                } else {
                    format!("{:?}", error.trap_code.expect("verified trap code"))
                };
                json_probe(case, mode, profile, driver, &kind, &error.to_string());
            }
            Ok(outcome) => panic!("correctness probe unexpectedly succeeded: {outcome:?}"),
        }
    }

    fn table_mutation_probe(args: &[String]) {
        assert_eq!(
            args.len(),
            7,
            "--scalar-table-probe FILE INPUT EXPECTED_A EXPECTED_B MODE PROFILE"
        );
        let bytes = std::fs::read(&args[1]).unwrap();
        let input = args[2].parse::<i32>().unwrap();
        let expected_a = args[3].parse::<i32>().unwrap();
        let expected_b = args[4].parse::<i32>().unwrap();
        let mode = args[5].as_str();
        let profile = args[6].as_str();
        let engine = configured_engine(mode, profile, false)
            .unwrap_or_else(|reason| panic!("invalid scalar-driver configuration: {reason}"));
        let module = engine.compile(&bytes).unwrap();
        let (mut store, instance) = instantiate(&engine, &module, Driver::TableA);
        let run = export(&store, instance, "run");
        let actual_a = returned_i32(&mut store, run, &[Value::I32(input)]);
        assert_eq!(actual_a, expected_a, "initial table target oracle mismatch");
        let set_target = export(&store, instance, "set_target");
        no_result(&mut store, set_target, &[Value::I32(1)]);
        let actual_b = returned_i32(&mut store, run, &[Value::I32(input)]);
        assert_eq!(actual_b, expected_b, "mutated table target oracle mismatch");
        no_result(&mut store, set_target, &[Value::I32(0)]);
        let restored_a = returned_i32(&mut store, run, &[Value::I32(input)]);
        assert_eq!(
            restored_a, expected_a,
            "restored table target oracle mismatch"
        );
        println!(
            "{{\"schema_version\":2,\"case\":\"scalar_table_mutation\",\"driver\":\"table_mutation\",\"diagnostic\":true,\"status\":\"mutation_verified\",\"input\":{},\"target_a\":{},\"target_b\":{},\"target_a_after_restore\":{},\"mode\":{},\"profile\":{}}}",
            quote(&format!("i32:{input}")),
            quote(&format!("i32:{actual_a}")),
            quote(&format!("i32:{actual_b}")),
            quote(&format!("i32:{restored_a}")),
            quote(mode),
            quote(profile),
        );
    }

    #[cfg(feature = "performance-counters")]
    fn scalar_diagnostic(args: &[String]) {
        assert_eq!(
            args.len(),
            8,
            "--scalar-diagnostic FILE CASE INPUT EXPECTED MODE PROFILE DRIVER"
        );
        let bytes = std::fs::read(&args[1]).unwrap();
        let case = args[2].as_str();
        assert!(
            !case.is_empty()
                && case
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        );
        let input = args[3].parse::<i32>().unwrap();
        let expected = args[4].parse::<i32>().unwrap();
        let requested_mode = args[5].as_str();
        assert_eq!(requested_mode, "compact", "diagnostic requires explicit Compact mode");
        let profile = args[6].as_str();
        let driver = Driver::parse(&args[7]).unwrap_or_else(|| panic!("unknown scalar driver {}", args[7]));
        assert!(
            matches!(driver, Driver::Plain | Driver::HostTick | Driver::TableA | Driver::TableB),
            "scalar diagnostic only accepts result-producing drivers"
        );

        let engine = configured_engine(requested_mode, profile, false)
            .unwrap_or_else(|reason| panic!("invalid scalar-driver configuration: {reason}"));
        assert_eq!(
            engine.execution_policy(),
            ExecutionPolicy::Compact,
            "requested Compact policy did not resolve to Compact"
        );
        let module = engine.compile(&bytes).unwrap();

        // Check the registry oracle on disposable state before collecting any
        // counters. Diagnostic setup is not part of the measured protocol.
        let (mut check_store, check_instance) = instantiate(&engine, &module, driver);
        check_call(&mut check_store, check_instance, driver, input, expected);

        let (mut store, instance) = instantiate(&engine, &module, driver);
        let func = export(&store, instance, "run");
        store.reset_performance_counters();
        let result = returned_i32(&mut store, func, &[Value::I32(input)]);
        assert_eq!(result, expected, "scalar diagnostic oracle mismatch");
        let counters = store.performance_counters().to_json();
        println!(
            "{{\"schema\":\"wtime-compact-scalar-diagnostic-v1\",\"diagnostic\":true,\"instrumented\":true,\"case\":{},\"input\":{},\"expected\":{},\"result\":{},\"driver\":{},\"profile\":{},\"requested_mode\":{},\"resolved_mode\":\"compact\",\"counters\":{}}}",
            quote(case),
            quote(&format!("i32:{input}")),
            quote(&format!("i32:{expected}")),
            quote(&format!("i32:{result}")),
            quote(&args[7]),
            quote(profile),
            quote(requested_mode),
            counters,
        );
    }

    #[cfg(feature = "performance-counters")]
    fn scalar_cache_diagnostic(args: &[String]) {
        assert_eq!(
            args.len(),
            8,
            "--scalar-cache-diagnostic FILE CASE INPUT EXPECTED MODE PROFILE DRIVER"
        );
        let bytes = std::fs::read(&args[1]).unwrap();
        let case = args[2].as_str();
        assert!(
            !case.is_empty()
                && case
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        );
        let input = args[3].parse::<i32>().unwrap();
        let expected = args[4].parse::<i32>().unwrap();
        let requested_mode = args[5].as_str();
        assert_eq!(requested_mode, "compact", "cache diagnostic requires explicit Compact mode");
        let profile = args[6].as_str();
        let driver = Driver::parse(&args[7]).unwrap_or_else(|| panic!("unknown scalar driver {}", args[7]));
        assert!(
            matches!(driver, Driver::Plain | Driver::HostTick | Driver::TableA | Driver::TableB),
            "scalar cache diagnostic only accepts result-producing drivers"
        );

        let engine = configured_engine(requested_mode, profile, false)
            .unwrap_or_else(|reason| panic!("invalid scalar-driver configuration: {reason}"));
        assert_eq!(
            engine.execution_policy(),
            ExecutionPolicy::Compact,
            "requested Compact policy did not resolve to Compact"
        );
        let module = engine.compile(&bytes).unwrap();

        // Verify the independent registry result on disposable state before
        // collecting cache counters. This call is excluded from the counters.
        let (mut check_store, check_instance) = instantiate(&engine, &module, driver);
        check_call(&mut check_store, check_instance, driver, input, expected);

        // Keep this Store and instance alive across both calls: a hit on the
        // second invocation is the diagnostic proof for a stable call site.
        let (mut store, instance) = instantiate(&engine, &module, driver);
        let func = export(&store, instance, "run");
        store.reset_performance_counters();
        let first = returned_i32(&mut store, func, &[Value::I32(input)]);
        assert_eq!(first, expected, "first scalar cache diagnostic oracle mismatch");
        let second = returned_i32(&mut store, func, &[Value::I32(input)]);
        assert_eq!(second, expected, "second scalar cache diagnostic oracle mismatch");
        let counters = store.performance_counters().to_json();
        let input_text = quote(&format!("i32:{input}"));
        let expected_text = quote(&format!("i32:{expected}"));
        let first_text = quote(&format!("i32:{first}"));
        let second_text = quote(&format!("i32:{second}"));
        println!(
            "{{\"schema\":\"wtime-compact-scalar-cache-diagnostic-v1\",\"diagnostic\":true,\"instrumented\":true,\"case\":{},\"input\":{},\"expected\":{},\"result\":{},\"invocations\":2,\"results\":[{},{}],\"driver\":{},\"profile\":{},\"requested_mode\":{},\"resolved_mode\":\"compact\",\"counters\":{}}}",
            quote(case), input_text, expected_text, second_text, first_text, second_text,
            quote(&args[7]), quote(profile), quote(requested_mode), counters,
        );
    }


    #[cfg(feature = "performance-counters")]
    fn simd_diagnostic(args: &[String]) {
        assert_eq!(args.len(), 7, "--simd-diagnostic FILE CASE TYPED_INPUT ALLOWED_CSV MODE PROFILE");
        let bytes=std::fs::read(&args[1]).unwrap();
        let case=&args[2];
        assert!(!case.is_empty() && case.bytes().all(|c| c.is_ascii_alphanumeric() || c==b'_' || c==b'-'));
        let input=super::super::parse_values(&args[3]);
        let mut allowed=args[4].split(',').map(|v| v.parse::<i32>().unwrap()).collect::<Vec<_>>();
        assert!(!allowed.is_empty() && allowed.len()<=16);
        allowed.sort_unstable();allowed.dedup();
        assert_eq!(args[5], "compact");
        let engine=configured_engine(&args[5],&args[6],false).unwrap();
        assert_eq!(engine.execution_policy(),ExecutionPolicy::Compact);
        let module=engine.compile(&bytes).unwrap();
        let (mut disposable,instance)=instantiate(&engine,&module,Driver::Plain);
        let function=export(&disposable,instance,"run");
        let checked=returned_i32(&mut disposable,function,&input);
        assert!(allowed.contains(&checked),"disposable allowed-set oracle mismatch");
        let (mut store,instance)=instantiate(&engine,&module,Driver::Plain);
        let function=export(&store,instance,"run");
        store.reset_performance_counters();
        let result=returned_i32(&mut store,function,&input);
        assert!(allowed.contains(&result),"diagnostic allowed-set oracle mismatch");
        let counters=store.performance_counters().to_json();
        let allowed_json=allowed.iter().map(i32::to_string).collect::<Vec<_>>().join(",");
        let memory=if matches!(instance.export(&store,"memory"),Ok(External::Memory(_))) { super::super::typed_memory_fields(&store,instance) } else { String::new() };
        println!("{{\"schema\":\"wtime-compact-simd-diagnostic-v1\",\"diagnostic\":true,\"instrumented\":true,\"case\":{},\"input\":{},\"allowed_results\":[{}],\"result\":{},\"requested_mode\":\"compact\",\"resolved_mode\":\"compact\",\"profile\":{},\"counters\":{}{} }}",quote(case),quote(&args[3]),allowed_json,result,quote(&args[6]),counters,memory);
    }

    pub fn try_main(args: &[String]) -> bool {
        match args.first().map(String::as_str) {
            #[cfg(feature = "performance-counters")]
            Some("--simd-diagnostic") => { simd_diagnostic(args); return true; },
            Some("--scalar-table-probe") => {
                table_mutation_probe(args);
                return true;
            }
            #[cfg(feature = "performance-counters")]
            Some("--scalar-diagnostic") => {
                scalar_diagnostic(args);
                return true;
            }
            #[cfg(feature = "performance-counters")]
            Some("--scalar-cache-diagnostic") => {
                scalar_cache_diagnostic(args);
                return true;
            }
            Some("--scalar-driver") => {}
            _ => return false,
        }
        assert_eq!(
            args.len(),
            10,
            "--scalar-driver FILE CASE INPUT EXPECTED STAGE BATCH MODE PROFILE DRIVER"
        );
        let bytes = std::fs::read(&args[1]).unwrap();
        let case = args[2].as_str();
        assert!(
            !case.is_empty()
                && case
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        );
        let input = args[3].parse::<i32>().unwrap();
        let expected = args[4].parse::<i32>().unwrap();
        let stage = args[5].as_str();
        let requested_batch = args[6].parse::<usize>().unwrap();
        assert!(requested_batch <= 100_000);
        let mode = args[7].as_str();
        let profile = args[8].as_str();
        let Some(driver) = Driver::parse(&args[9]) else {
            panic!("unknown scalar driver {}", args[9]);
        };
        assert!(
            [
                "prepare",
                "instantiate",
                "first_call",
                "warm_call",
                "load_and_call"
            ]
            .contains(&stage),
            "unsupported scalar-driver stage {stage}"
        );
        if driver.is_probe() {
            let bounded_depth = matches!(driver, Driver::DepthProbe | Driver::TailProbe);
            let engine = configured_engine(mode, profile, bounded_depth)
                .unwrap_or_else(|reason| panic!("invalid scalar-driver configuration: {reason}"));
            let module = engine.compile(&bytes).unwrap();
            probe(
                &engine, &module, case, mode, profile, driver, input, expected,
            );
            return true;
        }
        assert!(
            driver == Driver::Plain
                || driver == Driver::HostTick
                || driver.table_target().is_some()
        );
        let engine = configured_engine(mode, profile, false)
            .unwrap_or_else(|reason| panic!("invalid scalar-driver configuration: {reason}"));
        let module = engine.compile(&bytes).unwrap();

        // Validate the oracle before timing. This preflight state is discarded.
        let (mut check_store, check_instance) = instantiate(&engine, &module, driver);
        let preflight_result =
            check_call(&mut check_store, check_instance, driver, input, expected);

        let (batch, elapsed, result) = match stage {
            "prepare" => {
                // A new Engine keeps canonical interning work out of the timed compile.
                let prepare_engine =
                    configured_engine(mode, profile, false).unwrap_or_else(|reason| {
                        panic!("invalid scalar-driver configuration: {reason}")
                    });
                let start = Instant::now();
                let prepared = prepare_engine.compile(black_box(&bytes)).unwrap();
                let ns = start.elapsed().as_nanos() as f64;
                black_box(prepared);
                (1, ns, preflight_result)
            }
            "instantiate" => {
                let mut store = Store::new(&engine);
                let linker = linker(&engine, &mut store, driver);
                let start = Instant::now();
                let instance = linker.instantiate(&mut store, &module).unwrap();
                let ns = start.elapsed().as_nanos() as f64;
                // Table mutation follows the timed instantiate boundary.
                if let Some(target) = driver.table_target() {
                    let set_target = export(&store, instance, "set_target");
                    no_result(&mut store, set_target, &[Value::I32(target)]);
                }
                let actual = check_call(&mut store, instance, driver, input, expected);
                (1, ns, actual)
            }
            "load_and_call" => {
                let load_engine =
                    configured_engine(mode, profile, false).unwrap_or_else(|reason| {
                        panic!("invalid scalar-driver configuration: {reason}")
                    });
                let start = Instant::now();
                let timed_module = load_engine.compile(black_box(&bytes)).unwrap();
                let (mut store, instance) = instantiate(&load_engine, &timed_module, driver);
                let func = export(&store, instance, "run");
                let actual = invoke_timed(&mut store, func, input);
                let ns = start.elapsed().as_nanos() as f64;
                assert_eq!(actual, expected);
                (1, ns, actual)
            }
            "first_call" => {
                let (mut store, instance) = instantiate(&engine, &module, driver);
                let func = export(&store, instance, "run");
                let start = Instant::now();
                let actual = invoke_timed(&mut store, func, input);
                let ns = start.elapsed().as_nanos() as f64;
                assert_eq!(actual, expected);
                (1, ns, actual)
            }
            "warm_call" => {
                let (mut store, instance) = instantiate(&engine, &module, driver);
                let func = export(&store, instance, "run");
                for _ in 0..5 {
                    assert_eq!(invoke_timed(&mut store, func, input), expected);
                }
                let batch = if requested_batch == 0 {
                    let (one_ns, _) =
                        measured(1, || invoke_timed(&mut store, func, input), expected);
                    (20_000_000.0 / one_ns).ceil().clamp(1.0, 100_000.0) as usize
                } else {
                    requested_batch
                };
                let (elapsed, actual) =
                    measured(batch, || invoke_timed(&mut store, func, input), expected);
                (batch, elapsed, actual)
            }
            _ => unreachable!(),
        };
        json_timing(
            case, stage, input, expected, result, batch, elapsed, mode, profile, driver,
        );
        true
    }
}

pub use current::try_main;
