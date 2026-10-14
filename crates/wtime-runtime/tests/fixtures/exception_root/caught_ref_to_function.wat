;; SPDX-License-Identifier: CC0-1.0
;; Throw a fresh GC object; catch depth 0 exits the function and returns it.
(module
  (type $node (struct (field i32)))
  (tag $boom (param (ref $node)))
  (func (export "run") (result (ref $node))
    (try_table (catch $boom 0)
      (throw $boom
        (struct.new $node (i32.const 91))))
    (unreachable))
  (func (export "read") (param (ref $node)) (result i32)
    (struct.get $node 0 (local.get 0))))
