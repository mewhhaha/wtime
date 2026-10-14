(module
  (func (export "i32_eq_br") (param $a i32) (param $b i32) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i32.eq
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i32_eq_if") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.eq
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i32_ne_br") (param $a i32) (param $b i32) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i32.ne
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i32_ne_if") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.ne
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i32_lt_s_br") (param $a i32) (param $b i32) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i32.lt_s
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i32_lt_s_if") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.lt_s
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i32_lt_u_br") (param $a i32) (param $b i32) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i32.lt_u
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i32_lt_u_if") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.lt_u
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i32_gt_s_br") (param $a i32) (param $b i32) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i32.gt_s
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i32_gt_s_if") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.gt_s
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i32_gt_u_br") (param $a i32) (param $b i32) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i32.gt_u
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i32_gt_u_if") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.gt_u
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i32_le_s_br") (param $a i32) (param $b i32) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i32.le_s
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i32_le_s_if") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.le_s
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i32_le_u_br") (param $a i32) (param $b i32) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i32.le_u
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i32_le_u_if") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.le_u
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i32_ge_s_br") (param $a i32) (param $b i32) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i32.ge_s
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i32_ge_s_if") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.ge_s
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i32_ge_u_br") (param $a i32) (param $b i32) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i32.ge_u
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i32_ge_u_if") (param $a i32) (param $b i32) (result i32)
    local.get $a
    local.get $b
    i32.ge_u
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i64_eq_br") (param $a i64) (param $b i64) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i64.eq
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i64_eq_if") (param $a i64) (param $b i64) (result i32)
    local.get $a
    local.get $b
    i64.eq
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i64_ne_br") (param $a i64) (param $b i64) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i64.ne
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i64_ne_if") (param $a i64) (param $b i64) (result i32)
    local.get $a
    local.get $b
    i64.ne
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i64_lt_s_br") (param $a i64) (param $b i64) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i64.lt_s
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i64_lt_s_if") (param $a i64) (param $b i64) (result i32)
    local.get $a
    local.get $b
    i64.lt_s
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i64_lt_u_br") (param $a i64) (param $b i64) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i64.lt_u
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i64_lt_u_if") (param $a i64) (param $b i64) (result i32)
    local.get $a
    local.get $b
    i64.lt_u
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i64_gt_s_br") (param $a i64) (param $b i64) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i64.gt_s
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i64_gt_s_if") (param $a i64) (param $b i64) (result i32)
    local.get $a
    local.get $b
    i64.gt_s
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i64_gt_u_br") (param $a i64) (param $b i64) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i64.gt_u
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i64_gt_u_if") (param $a i64) (param $b i64) (result i32)
    local.get $a
    local.get $b
    i64.gt_u
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i64_le_s_br") (param $a i64) (param $b i64) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i64.le_s
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i64_le_s_if") (param $a i64) (param $b i64) (result i32)
    local.get $a
    local.get $b
    i64.le_s
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i64_le_u_br") (param $a i64) (param $b i64) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i64.le_u
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i64_le_u_if") (param $a i64) (param $b i64) (result i32)
    local.get $a
    local.get $b
    i64.le_u
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i64_ge_s_br") (param $a i64) (param $b i64) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i64.ge_s
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i64_ge_s_if") (param $a i64) (param $b i64) (result i32)
    local.get $a
    local.get $b
    i64.ge_s
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
  (func (export "i64_ge_u_br") (param $a i64) (param $b i64) (result i32)
    block (result i32)
      i32.const 71
      local.get $a
      local.get $b
      i64.ge_u
      (@metadata.code.branch_hint "\00")
      br_if 0
      drop
      i32.const -13
    end)
  (func (export "i64_ge_u_if") (param $a i64) (param $b i64) (result i32)
    local.get $a
    local.get $b
    i64.ge_u
    (@metadata.code.branch_hint "\00")
    if (result i32)
      i32.const 71
    else
      i32.const -13
    end)
)
