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
  (func (export "run") (type $run) call $leaf)
  (func (export "resume") (param (ref $cont)) (result i32)
    i32.const 9
    local.get 0
    resume $cont))
