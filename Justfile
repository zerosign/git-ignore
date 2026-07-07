# Build in debug mode (Fast iteration, avoids aws-lc-rs)
build:
    cargo build

# Run in debug mode
run *args:
    cargo run -- {{args}}

# Run tests (Safe for mold linker)
test:
    cargo test -- --test-threads=1

# Run clippy
lint:
    cargo clippy -- -D warnings

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

# Install shell completions (requires sudo for some locations)
install-completions:
    @echo "Installing completions..."
    # Bash
    mkdir -p ~/.local/share/bash-completion/completions
    cp resources/git-ignore.bash ~/.local/share/bash-completion/completions/git-ignore
    # Fish
    mkdir -p ~/.config/fish/completions
    cp resources/git-ignore.fish ~/.config/fish/completions/git-ignore.fish
    # Zsh (local user directory, ensure it's in your fpath)
    mkdir -p ~/.zsh/completions
    cp resources/_git-ignore ~/.zsh/completions/_git-ignore
    @echo "Done. Note: For Zsh, ensure ~/.zsh/completions is in your fpath in .zshrc."
