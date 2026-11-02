set -eu
# usage: build.sh <name>  -> /work/obsspeed/bin/<name>/quod
cd /src
cargo build --release -p quod-cli 2>&1 | tail -3
mkdir -p /work/obsspeed/bin/$1
cp target/release/quod /work/obsspeed/bin/$1/quod
ls -la /work/obsspeed/bin/$1/
