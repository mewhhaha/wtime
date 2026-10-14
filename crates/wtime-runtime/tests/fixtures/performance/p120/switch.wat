;; SPDX-License-Identifier: CC0-1.0
(module
 (type $result (func (result i32)))
 (type $c0 (cont $result))
 (type $worker_f (func (param (ref $c0)) (result i32)))
 (type $cw (cont $worker_f))
 (type $switcher_f (func (param (ref $cw)) (result i32)))
 (type $cs (cont $switcher_f))
 (tag $yield (type $result))
 (func $worker (type $worker_f)
  local.get 0 resume $c0 i32.const 2 i32.add)
 (func $switcher (type $switcher_f) (local i32)
  i32.const 7 local.set 1
  local.get 0 switch $cw $yield
  local.get 1)
 (elem declare func $worker $switcher)
 (func (export "run") (type $result)
  (resume $c0 (on $yield switch)
   (cont.bind $cs $c0
    (cont.new $cw (ref.func $worker))
    (cont.new $cs (ref.func $switcher))))))
