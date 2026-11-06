target := "thumbv7em-none-eabihf"
binary := "preamp_daisy"

default: format clippy

check-format:
    cargo fmt --all -- --check

format:
    cargo fmt --all

# The Daisy Seed's MCU, for probe-rs.
chip := "STM32H750IBKx"

# Crates that only build for the Daisy target, and the no_std crates that
# also get checked for it.
firmware_crates := "-p preamp_daisy -p daisy_runtime -p euterpe_daisy -p demo_daisy -p euterpe_bench_qemu -p euterpe_bench_daisy"
no_std_crates := "-p euterpe_core -p euterpe_dsp -p euterpe_demo -p euterpe_pedals -p euterpe_registry -p euterpe_bench -p dsp -p preamp"
host_excludes := "--exclude preamp_daisy --exclude daisy_runtime --exclude euterpe_daisy --exclude demo_daisy --exclude euterpe_bench_qemu --exclude euterpe_bench_daisy"

# Host crates on the host, firmware and no_std crates on the Daisy target.
clippy: clippy-host clippy-firmware

clippy-host:
    cargo clippy --workspace {{host_excludes}} --all-targets -- -D warnings

clippy-firmware:
    cargo clippy {{firmware_crates}} {{no_std_crates}} --target {{target}} -- -D warnings

# Every crate type-checks for the target it runs on.
check:
    cargo check --workspace {{host_excludes}} --all-targets
    cargo check {{firmware_crates}} {{no_std_crates}} --target {{target}}

# Tests for every host crate.
test:
    cargo test --workspace {{host_excludes}}

update:
    cargo update

build:
    cargo build --release -p {{binary}} --target {{target}}

# Flash a firmware crate over DFU, e.g. `just flash demo_daisy`.
flash bin=binary:
    cargo build --release -p {{bin}} --target {{target}}
    rust-objcopy -O binary target/{{target}}/release/{{bin}} target/{{bin}}.bin
    dfu-suffix --vid 0x0483 --pid 0xdf11 --add target/{{bin}}.bin
    dfu-util -a 0 -s 0x08000000:leave -D target/{{bin}}.bin -d ,0483:df11 || true

# Flash any registered device to the Daisy, e.g. `just flash-device demo.rig`.
flash-device device:
    EUTERPE_DEVICE={{device}} just flash demo_daisy

# Run a device on the Daisy via a debug probe, with cycle stats over RTT.
probe-run device="demo.drive":
    EUTERPE_DEVICE={{device}} cargo build --release -p demo_daisy --features rtt --target {{target}}
    probe-rs run --chip {{chip}} target/{{target}}/release/demo_daisy

# Bench every device on the Daisy via a debug probe, next to QEMU's counts.
board-bench:
    cargo build --release -p euterpe_bench_daisy --target {{target}}
    probe-rs run --chip {{chip}} target/{{target}}/release/euterpe_bench_daisy | tee target/board-bench.log
    just bench --board-log target/board-bench.log

# Instruction counts per block on an emulated Cortex-M7, checked against the
# baseline. Needs qemu-system-arm, cc + glib headers, and llvm-tools.
bench *args:
    cargo run -q --release -p euterpe_cli -- bench --baseline benches/qemu-baseline.txt {{args}}

# Accept the current instruction counts as the new baseline.
bench-baseline:
    cargo run -q --release -p euterpe_cli -- bench --write-baseline benches/qemu-baseline.txt

# Play a device through the audio interface, e.g. `just play demo.drive`.
play *args:
    cargo run -q --release -p euterpe_desktop -- {{args}}

dev:
    cargo run -p preamp_desktop

# Euterpe CLI, e.g. `just euterpe render demo.drive --gen sine:220 --out out.wav`
euterpe *args:
    cargo run -q --release -p euterpe_cli -- {{args}}

# Test the Euterpe crates only (no hardware or desktop dependencies)
test-euterpe:
    cargo test -p euterpe_core -p euterpe_macros -p euterpe_dsp -p euterpe_offline -p euterpe_cli -p euterpe_demo -p euterpe_pedals -p euterpe_registry -p euterpe_fixtures

# Guitar test fixtures, e.g. `just fixtures fetch`, `just fixtures excerpt`, `just fixtures list`
fixtures *args:
    cargo run -q --release -p euterpe_fixtures --bin euterpe-fixtures -- {{args}}

# Kani proofs (index math, ring arithmetic, fastmath bounds). Needs
# `cargo install --locked kani-verifier@0.68.0 && cargo kani setup`.
kani:
    cargo kani -p euterpe_core
    cargo kani -p euterpe_dsp
