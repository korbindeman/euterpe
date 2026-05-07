target := "thumbv7em-none-eabihf"
binary := "preamp_daisy"

default: format clippy

check-format:
    cargo fmt --all -- --check

format:
    cargo fmt --all

clippy:
    cargo clippy --workspace --target {{target}} -- -D warnings

update:
    cargo update

build:
    cargo build --release -p {{binary}} --target {{target}}

flash: build
    rust-objcopy -O binary target/{{target}}/release/{{binary}} target/{{binary}}.bin
    dfu-suffix --vid 0x0483 --pid 0xdf11 --add target/{{binary}}.bin
    dfu-util -a 0 -s 0x08000000:leave -D target/{{binary}}.bin -d ,0483:df11 || true

dev:
    cargo run -p preamp_desktop
