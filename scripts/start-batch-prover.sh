cd ../service
echo "Starting batch prover, log file: batch-prover.log"
SCHEDULER_ADDR="http://10.0.0.4:50061" RUST_LOG=debug CACHE_DIR=/mnt/zkevmproverstorage/zkevmfileshare/eigen-prover/prover/cache BASEDIR=/mnt/zkevmproverstorage/zkevmfileshare/eigen-prover/prover/data WORK_BASE="/mnt/zkevmproverstorage/zkevmfileshare/eigen-zkvm/starkjs" FORCE_BIT=18 RUSTFLAGS="-C target-cpu=native" RUST_MIN_STACK=2073741821 CIRCOMLIB=${WORK_BASE}/node_modules/circomlib/circuits STARK_VERIFIER_GL=$WORK_BASE/node_modules/pil-stark/circuits.gl STARK_VERIFIER_BN128=$WORK_BASE/node_modules/pil-stark/circuits.bn128 nohup cargo run --bin batch-prover --release >> batch-prover.log 2>&1 &