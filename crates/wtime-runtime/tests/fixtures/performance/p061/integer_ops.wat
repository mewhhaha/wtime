;; SPDX-License-Identifier: MIT
;; P061 core integer opcode coverage fixture; authored for this repository.
(module
  (func (export "i32_eqz") (param $0 i32) (result i32)
    local.get 0
    i32.eqz)
  (func (export "i64_eqz") (param $0 i64) (result i32)
    local.get 0
    i64.eqz)
  (func (export "i32_eq") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.eq)
  (func (export "i32_ne") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.ne)
  (func (export "i32_lt_s") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.lt_s)
  (func (export "i32_lt_u") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.lt_u)
  (func (export "i32_gt_s") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.gt_s)
  (func (export "i32_gt_u") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.gt_u)
  (func (export "i32_le_s") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.le_s)
  (func (export "i32_le_u") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.le_u)
  (func (export "i32_ge_s") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.ge_s)
  (func (export "i32_ge_u") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.ge_u)
  (func (export "i64_eq") (param $0 i64) (param $1 i64) (result i32)
    local.get 0
    local.get 1
    i64.eq)
  (func (export "i64_ne") (param $0 i64) (param $1 i64) (result i32)
    local.get 0
    local.get 1
    i64.ne)
  (func (export "i64_lt_s") (param $0 i64) (param $1 i64) (result i32)
    local.get 0
    local.get 1
    i64.lt_s)
  (func (export "i64_lt_u") (param $0 i64) (param $1 i64) (result i32)
    local.get 0
    local.get 1
    i64.lt_u)
  (func (export "i64_gt_s") (param $0 i64) (param $1 i64) (result i32)
    local.get 0
    local.get 1
    i64.gt_s)
  (func (export "i64_gt_u") (param $0 i64) (param $1 i64) (result i32)
    local.get 0
    local.get 1
    i64.gt_u)
  (func (export "i64_le_s") (param $0 i64) (param $1 i64) (result i32)
    local.get 0
    local.get 1
    i64.le_s)
  (func (export "i64_le_u") (param $0 i64) (param $1 i64) (result i32)
    local.get 0
    local.get 1
    i64.le_u)
  (func (export "i64_ge_s") (param $0 i64) (param $1 i64) (result i32)
    local.get 0
    local.get 1
    i64.ge_s)
  (func (export "i64_ge_u") (param $0 i64) (param $1 i64) (result i32)
    local.get 0
    local.get 1
    i64.ge_u)
  (func (export "i32_clz") (param $0 i32) (result i32)
    local.get 0
    i32.clz)
  (func (export "i32_ctz") (param $0 i32) (result i32)
    local.get 0
    i32.ctz)
  (func (export "i32_popcnt") (param $0 i32) (result i32)
    local.get 0
    i32.popcnt)
  (func (export "i64_clz") (param $0 i64) (result i64)
    local.get 0
    i64.clz)
  (func (export "i64_ctz") (param $0 i64) (result i64)
    local.get 0
    i64.ctz)
  (func (export "i64_popcnt") (param $0 i64) (result i64)
    local.get 0
    i64.popcnt)
  (func (export "i32_add") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.add)
  (func (export "i32_sub") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.sub)
  (func (export "i32_mul") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.mul)
  (func (export "i32_div_s") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.div_s)
  (func (export "i32_div_u") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.div_u)
  (func (export "i32_rem_s") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.rem_s)
  (func (export "i32_rem_u") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.rem_u)
  (func (export "i32_and") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.and)
  (func (export "i32_or") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.or)
  (func (export "i32_xor") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.xor)
  (func (export "i32_shl") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.shl)
  (func (export "i32_shr_s") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.shr_s)
  (func (export "i32_shr_u") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.shr_u)
  (func (export "i32_rotl") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.rotl)
  (func (export "i32_rotr") (param $0 i32) (param $1 i32) (result i32)
    local.get 0
    local.get 1
    i32.rotr)
  (func (export "i64_add") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.add)
  (func (export "i64_sub") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.sub)
  (func (export "i64_mul") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.mul)
  (func (export "i64_div_s") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.div_s)
  (func (export "i64_div_u") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.div_u)
  (func (export "i64_rem_s") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.rem_s)
  (func (export "i64_rem_u") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.rem_u)
  (func (export "i64_and") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.and)
  (func (export "i64_or") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.or)
  (func (export "i64_xor") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.xor)
  (func (export "i64_shl") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.shl)
  (func (export "i64_shr_s") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.shr_s)
  (func (export "i64_shr_u") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.shr_u)
  (func (export "i64_rotl") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.rotl)
  (func (export "i64_rotr") (param $0 i64) (param $1 i64) (result i64)
    local.get 0
    local.get 1
    i64.rotr)
  (func (export "i32_wrap_i64") (param $0 i64) (result i32)
    local.get 0
    i32.wrap_i64)
  (func (export "i64_extend_i32_s") (param $0 i32) (result i64)
    local.get 0
    i64.extend_i32_s)
  (func (export "i64_extend_i32_u") (param $0 i32) (result i64)
    local.get 0
    i64.extend_i32_u)
  (func (export "i32_extend8_s") (param $0 i32) (result i32)
    local.get 0
    i32.extend8_s)
  (func (export "i32_extend16_s") (param $0 i32) (result i32)
    local.get 0
    i32.extend16_s)
  (func (export "i64_extend8_s") (param $0 i64) (result i64)
    local.get 0
    i64.extend8_s)
  (func (export "i64_extend16_s") (param $0 i64) (result i64)
    local.get 0
    i64.extend16_s)
  (func (export "i64_extend32_s") (param $0 i64) (result i64)
    local.get 0
    i64.extend32_s)
)
