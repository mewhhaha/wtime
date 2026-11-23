#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
case "${1:-core}" in
 core) cargo test --locked --workspace --exclude wtime-conformance ;;
 feature) cargo run --locked -p wtime-conformance -- run --profile "${PROFILE:-stable-core}" --suite "${SUITE:?set SUITE}" --filter "${FILTER:-all}" --report "target/conformance/${SUITE}.json" ;;
 full)
  python3 -B tools/checks/fetch_corpus.py
  cargo run --locked --release -p wtime-conformance --features opcode-coverage -- run --profile mvp --suite mvp --filter all --report target/conformance/mvp.json
  cargo run --locked --release -p wtime-conformance --features opcode-coverage -- run --profile stable-core --suite core3 --filter all --report target/conformance/core3.json
  cargo run --locked --release -p wtime-conformance --features opcode-coverage -- run --profile all-phase3-core --suite phase3 --filter all --report target/conformance/phase3.json
  cargo run --locked --release -p wtime-conformance --features opcode-coverage -- run --profile all-phase3-core --suite interactions --filter all --report target/conformance/interactions.json
  ;;
 allocation)
  cargo build --locked -p wtime-conformance --bin wtime-allocation-check
  python3 -B tools/checks/allocation_failure.py --worker target/debug/wtime-allocation-check --report target/conformance/allocation-failure.json
  ;;
 budget) cargo build --locked --release -p wtime-cli; wc -c target/release/wtime-cli ;;
 miri) cargo miri test --locked -p wtime-types ;;
 oracle) echo 'Optional oracle not configured' >&2; exit 77 ;;
 *) echo "unknown lane: $1" >&2; exit 2 ;;
esac
