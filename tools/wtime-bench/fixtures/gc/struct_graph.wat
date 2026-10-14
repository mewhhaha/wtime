(module
  (rec (type $node (struct (field i32) (field (ref null $node)))))
  (func (export "run") (param $count i32) (result i32)
    (local $index i32)
    (local $sum i32)
    (local $head (ref null $node))
    (local $cursor (ref null $node))
    (block $built
      (loop $build
        (br_if $built (i32.ge_u (local.get $index) (local.get $count)))
        (local.set $head
          (struct.new $node (local.get $index) (local.get $head)))
        (local.set $index (i32.add (local.get $index) (i32.const 1)))
        (br $build)))
    (local.set $cursor (local.get $head))
    (block $done
      (loop $scan
        (br_if $done (ref.is_null (local.get $cursor)))
        (local.set $sum
          (i32.add (local.get $sum) (struct.get $node 0 (local.get $cursor))))
        (local.set $cursor (struct.get $node 1 (local.get $cursor)))
        (br $scan)))
    (local.get $sum)))
