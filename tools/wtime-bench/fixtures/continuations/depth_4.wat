(module
  (type $run (func (result i32)))
  (type $step (func (param i32) (result i32)))
  (type $cont (cont $step))
  (tag $yield (type $step))
  (func $leaf (type $run) (local $saved i32)
    i32.const 1000
    local.set $saved
    i32.const 7
    suspend $yield
    local.get $saved
    i32.add)
  (func $frame_0 (type $run) (local $saved_0 i32)
    i32.const 1
    local.set $saved_0
    call $frame_1
    local.get $saved_0
    i32.add)
  (func $frame_1 (type $run) (local $saved_1 i32)
    i32.const 2
    local.set $saved_1
    call $frame_2
    local.get $saved_1
    i32.add)
  (func $frame_2 (type $run) (local $saved_2 i32)
    i32.const 3
    local.set $saved_2
    call $frame_3
    local.get $saved_2
    i32.add)
  (func $frame_3 (type $run) (local $saved_3 i32)
    i32.const 4
    local.set $saved_3
    call $leaf
    local.get $saved_3
    i32.add)
  (func (export "run") (type $run) call $frame_0)
  (func (export "resume") (param (ref $cont)) (result i32)
    i32.const 9
    local.get 0
    resume $cont))
