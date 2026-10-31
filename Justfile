# Default recipe: quick check
default: check

# Check host target
check:
    cargo check

# Check static musl target with performance feature set
check-musl:
    cargo check --target x86_64-unknown-linux-musl --no-default-features --features performance

# Check all targets and test configurations
check-all: check check-musl
    cargo check --tests

# Build in debug mode (Fast iteration, avoids aws-lc-rs)
build:
    cargo build

# Run in debug mode
run *args:
    cargo run -- {{args}}

# Run tests (Safe for mold linker)
test *args="":
    cargo test {{args}} -- --test-threads=1

# Run clippy
lint:
    cargo clippy --all-targets -- -D warnings

# Format code
fmt:
    cargo fmt

# Build a fully static musl binary for release (Includes aws-lc-rs for performance)
release-static:
    rustup target add x86_64-unknown-linux-musl
    cargo build --target x86_64-unknown-linux-musl --release \
        --no-default-features --features performance
    @echo "Static binary generated at: target/x86_64-unknown-linux-musl/release/git-ignore"

# Build and install the static binary to ~/.cargo/bin
install-static: release-static
    mkdir -p ~/.cargo/bin
    cp target/x86_64-unknown-linux-musl/release/git-ignore ~/.cargo/bin/git-ignore
    @echo "Static binary installed to: ~/.cargo/bin/git-ignore"

# Profile command syscalls using strace on the static musl release binary
bench-strace *args="update":
    strace -c ./target/x86_64-unknown-linux-musl/release/git-ignore {{args}}

# Build a standard optimized binary for a specific target (used by CI for macOS/Windows)
release-dynamic target:
    cargo build --target {{target}} --release \
        --no-default-features --features performance
    @echo "Binary generated at: target/{{target}}/release/git-ignore"

# Create a git tag and push to trigger the GitHub Actions release workflow
tag version:
    @echo "Tagging release v{{version}}..."
    git tag -a v{{version}} -m "Release v{{version}}"
    git push origin v{{version}}
    @echo "Tag pushed! GitHub Actions will now build and publish the release."

# Install shell completions for Fish, Bash, and Zsh
install-completions:
    @echo "Installing completions..."
    # Bash
    mkdir -p ~/.local/share/bash-completion/completions
    cargo run --quiet -- completion bash > ~/.local/share/bash-completion/completions/git-ignore
    # Fish
    mkdir -p ~/.config/fish/completions
    cargo run --quiet -- completion fish > ~/.config/fish/completions/git-ignore.fish
    # Zsh (local user directory, ensure it's in your fpath)
    mkdir -p ~/.zsh/completions
    cargo run --quiet -- completion zsh > ~/.zsh/completions/_git-ignore
    @echo "Done! Installed completions for fish, bash, and zsh."
