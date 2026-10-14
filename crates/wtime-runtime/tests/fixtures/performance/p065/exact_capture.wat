;; SPDX-License-Identifier: MIT
(module
 (type $object (struct (field i32)))
 (type $result (func (result i32)))
 (type $step (func (param (ref (exact $object))) (result i32)))
 (type $c1 (cont $step))
 (type $wrong (struct (field i64)))
 (import "provider" "yield" (tag $yield (result (ref (exact $object)))))
 (func (export "run") (type $result) (local $i i32)
  i32.const 6 i32.const 7 i32.add local.set $i i32.const 99
  suspend $yield
  drop local.get $i i32.add)
 (func (export "make") (result (ref (exact $object)))
  i32.const 77 struct.new $object)
 (func (export "get") (param (ref (exact $object))) (result i32)
  local.get 0 struct.get $object 0)
 (func (export "make_wrong") (result (ref $wrong))
  i64.const 88 struct.new $wrong)
 (func (export "resume") (param (ref $c1)) (param (ref (exact $object))) (result i32)
  local.get 1 local.get 0 resume $c1))
