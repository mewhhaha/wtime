(module
  (func $leaf (result i32)
    ;; Module-local body 0: PC 0 const1 (charge 3), PC 1 const2 (charge 4),
    ;; PC 2 i32.add is the fifth charge; PC 3 function end.
    i32.const 1
    i32.const 2
    i32.add)
  (func (export "run") (result i32)
    ;; Module-local body 1: PC 0 const7 (charge 1), PC 1 call $leaf (charge 2);
    ;; caller's 7 remains live while leaf reaches transient shared height 3.
    ;; PC 2 i32.add; PC 3 function end.
    i32.const 7
    call $leaf
    i32.add))
