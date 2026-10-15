(module
  (type $node (struct (field i32) (field (ref null $node))))
  (import "env" "collect" (func $collect (result i32)))
  (global $root (mut (ref null $node)) (ref.null $node))
  (func (export "init")
    (local $head (ref null $node)) (local $index i32)
    (block $build_done
      (loop $build
        (br_if $build_done (i32.ge_u (local.get $index) (i32.const 256)))
        (local.set $head
          (struct.new $node (local.get $index) (local.get $head)))
        (local.set $index (i32.add (local.get $index) (i32.const 1)))
        (br $build)))
    (global.set $root (local.get $head)))
  (func (export "run") (param $repetitions i32) (result i32)
    (local $sum i32) (local $iteration i32)
    (local $cursor (ref null $node))
    (local.set $cursor (global.get $root))
    (block $calls_done
      (loop $calls
        (br_if $calls_done (i32.ge_u (local.get $iteration) (local.get $repetitions)))
        (local.set $sum (i32.add (local.get $sum) (call $collect)))
        (local.set $iteration (i32.add (local.get $iteration) (i32.const 1)))
        (br $calls)))
    (block $scan_done
      (loop $scan
        (br_if $scan_done (ref.is_null (local.get $cursor)))
        (local.set $sum
          (i32.add (local.get $sum) (struct.get $node 0 (local.get $cursor))))
        (local.set $cursor (struct.get $node 1 (local.get $cursor)))
        (br $scan)))
    (local.get $sum))
  (func (export "clear")
    (global.set $root (ref.null $node)))
)
