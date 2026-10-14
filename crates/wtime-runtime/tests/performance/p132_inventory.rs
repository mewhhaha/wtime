#[test]
fn p132_eligible_functions_preserve_prepared_artifacts() {
    let engine =
        Engine::with_execution_policy(Config::default(), crate::ExecutionPolicy::Register).unwrap();
    let cases: &[(&str, &[u8])] = &[
        (
            "integer_p061",
            include_bytes!("../fixtures/performance/p061/integer_ops.wasm").as_slice(),
        ),
        (
            "float_ops_p062",
            include_bytes!("../fixtures/performance/p062/float_ops.wasm").as_slice(),
        ),
        (
            "float_loop_p062",
            include_bytes!("../fixtures/performance/p062/float_loop.wasm").as_slice(),
        ),
        (
            "double_loop_p062",
            include_bytes!("../fixtures/performance/p062/double_loop.wasm").as_slice(),
        ),
        (
            "mixed_slots_p062",
            include_bytes!("../fixtures/performance/p062/mixed_slots.wasm").as_slice(),
        ),
        (
            "memory_loads_p063",
            include_bytes!("../fixtures/performance/p063/scalar_loads.wasm").as_slice(),
        ),
        (
            "memory_stores_p063",
            include_bytes!("../fixtures/performance/p063/scalar_stores.wasm").as_slice(),
        ),
        (
            "memory_widths_p063",
            include_bytes!("../fixtures/performance/p063/scalar-widths.wasm").as_slice(),
        ),
        (
            "memory_prior_effects_p063",
            include_bytes!("../fixtures/performance/p063/prior-effects.wasm").as_slice(),
        ),
        (
            "memory_bindings_p063",
            include_bytes!("../fixtures/performance/p063/unshared-bindings.wasm").as_slice(),
        ),
        (
            "control_caller_p060",
            include_bytes!("../fixtures/performance/p060/multivalue_caller.wasm").as_slice(),
        ),
        (
            "control_table_p060",
            include_bytes!("../fixtures/performance/p060/multivalue_table.wasm").as_slice(),
        ),
        (
            "control_target_p060",
            include_bytes!("../fixtures/performance/p060/table_caller.wasm").as_slice(),
        ),
        (
            "call_direct_p064",
            include_bytes!("../fixtures/performance/p064/direct_call.wasm").as_slice(),
        ),
        (
            "call_multi_p064",
            include_bytes!("../fixtures/performance/p064/calls.wasm").as_slice(),
        ),
        (
            "call_host_p064",
            include_bytes!("../fixtures/performance/p064/host_call.wasm").as_slice(),
        ),
        (
            "capture_numeric_p065",
            include_bytes!("../fixtures/performance/p065/numeric_capture.wasm").as_slice(),
        ),
        (
            "capture_exact_p065",
            include_bytes!("../fixtures/performance/p065/exact_capture.wasm").as_slice(),
        ),
        (
            "capture_exception_p065",
            include_bytes!("../fixtures/performance/p065/numeric_exception.wasm").as_slice(),
        ),
        (
            "unsupported_guards_p065",
            include_bytes!("../fixtures/performance/p065/barrier_guards.wasm").as_slice(),
        ),
    ];
    let mut prepared_fixtures = 0usize;
    let mut total_prepared = 0usize;
    let mut total_declined = 0usize;
    for (name, bytes) in cases.iter().copied() {
        let module = engine.compile(bytes).unwrap();
        let body_count = module.data.inner.decoded().code.len();
        let prepared = module
            .data
            .register
            .as_ref()
            .expect("Register preparation result");
        let prepared_count = prepared.count();
        let declined_count = body_count - prepared_count;
        if prepared_count > 0 {
            prepared_fixtures += 1;
        }
        total_prepared += prepared_count;
        total_declined += declined_count;
        eprintln!(
            "P132_INVENTORY|fixture|name={name}|total_body_count={body_count}|prepared_count={prepared_count}|declined_count={declined_count}"
        );
        for body in 0..body_count {
            let Some(function) = prepared.function(body) else {
                eprintln!("P132_INVENTORY|body|name={name}|body={body}|status=None");
                continue;
            };
            let program = &function.lowered.program;
            let n = program.instruction_count();
            let (work, bytes) = program.costs();
            let edge_count = function.edges.iter().map(Vec::len).sum::<usize>();
            eprintln!(
                "P132_INVENTORY|body|name={name}|body={body}|status=Prepared|instruction_count={n}|slots={}|program_costs=({work},{bytes})|lowered_work={}|lowered_bytes={}|edge_count={edge_count}",
                program.slots(),
                function.lowered.work,
                function.lowered.retained_bytes
            );
            for pc in 0..n {
                let opcode = program
                    .opcode(pc)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "None".into());
                let operation = format!("{:?}", program.operation(pc));
                let recipe = program
                    .materialization(&module, pc)
                    .expect("bounded materialization recipe");
                let locals = format!("{:?}", recipe.locals);
                let operands = format!(
                    "{:?}",
                    recipe
                        .operands
                        .iter()
                        .map(|slot| (slot.slot, slot.ty))
                        .collect::<Vec<_>>()
                );
                eprintln!(
                    "P132_INVENTORY|pc|name={name}|body={body}|pc={pc}|opcode={opcode}|operation={operation}|locals={locals}|operands={operands}"
                );
                if let Some(constant) = program.constant(pc as u32) {
                    eprintln!(
                        "P132_INVENTORY|constant|name={name}|body={body}|index={pc}|value={constant:?}"
                    );
                }
                if let Some(argument) = program.memory_argument(pc as u32) {
                    eprintln!(
                        "P132_INVENTORY|memory_argument|name={name}|body={body}|index={pc}|memory={}|offset={}|align={}",
                        argument.memory, argument.offset, argument.align
                    );
                }
                for edge in &function.edges[pc] {
                    eprintln!(
                        "P132_INVENTORY|edge|name={name}|body={body}|pc={pc}|target={}|moves_steps={:?}|scratch_cost={}|move_costs={:?}",
                        edge.target,
                        edge.moves.steps(),
                        edge.moves.scratch_slots(),
                        edge.moves.costs()
                    );
                }
            }
        }
    }
    eprintln!(
        "P132_INVENTORY|summary|fixture_count={}|prepared_fixture_count={prepared_fixtures}|total_prepared={total_prepared}|total_declined={total_declined}",
        cases.len()
    );
    assert!(
        prepared_fixtures >= 8,
        "expected at least eight fixtures to prepare a body, got {prepared_fixtures}"
    );
    assert!(
        total_declined > 0,
        "expected unsupported guard fixture declines"
    );
}
