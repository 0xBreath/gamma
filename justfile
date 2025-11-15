# Set the default shell to bash
set shell := ["bash", "-cu"]

# Environment variables
set dotenv-load := true
set export

export PROGRAMS_DIR := "tests-ts/programs"

# Install Rust, Solana CLI, and Anchor with version checks
install-tools:
    # Check and install Rust
    @echo "Checking for Rust..."
    @if command -v rustc >/dev/null 2>&1; then \
        RUST_VERSION=$(rustc --version | awk '{print $2}'); \
        if [ "$RUST_VERSION" = "1.91.0" ]; then \
            echo "Rust 1.91.0 is already installed"; \
        else \
            echo "Rust is installed but version is $RUST_VERSION, not 1.91.0"; \
            echo "Installing Rust 1.91.0"; \
            rustup toolchain install 1.91.0; \
        fi; \
    else \
        echo "Installing nightly Rust 1.93.0"; \
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.91.0; \
        source "$HOME/.cargo/env"; \
    fi

    # Check and install Solana CLI
    @echo "Checking for Solana CLI..."
    @if command -v solana >/dev/null 2>&1; then \
        echo "Solana CLI is already installed. Version: $(solana --version)"; \
    else \
        echo "Installing Solana v2.3.0"; \
        sh -c "$(curl -sSfL https://release.anza.xyz/stable/install)"; \
        echo 'export PATH="$HOME/.local/share/solana/install/active_release/bin:$PATH"' >> ~/.zshrc; \
    fi
    @echo "Use Solana v2.3.0"
    @agave-install init 2.3.0

    # Install Anchor
    @echo "Checking for Anchor..."
    @if command -v avm >/dev/null 2>&1; then \
        echo "Anchor Version Manager already installed"; \
    else \
        echo "Installing Anchor..."; \
        cargo install --git https://github.com/coral-xyz/anchor avm --force; \
        avm install 0.31.1; \
    fi

    # Setup Anchor version
    @echo "Setting up Anchor version 0.31.1"
    @avm use 0.31.1

    # Verification
    @echo "Installation complete! Please restart your terminal or run 'source ~/.bashrc' (or ~/.zshrc if you use zsh)"
    @echo "Verify installations:"
    @echo "Rust: $(cargo --version 2>/dev/null || echo 'not found')"
    @echo "Solana: $(solana --version 2>/dev/null || echo 'not found')"
    @echo "Anchor: $(anchor --version 2>/dev/null || echo 'not found')"

# for whatever reason we can't compile without --no-idl even though they say it was fixed here by upgrading to 1.93.0:
# https://github.com/solana-foundation/anchor/issues/3947
# and here, maybe, with 0.31.0 release:
# https://www.anchor-lang.com/docs/updates/release-notes/0-31-0#address-constraint-with-non-const-expressions
# however none of these worked, so the build flag is there for now
build-local:
	@just install-tools

	# Exit on error
	@set -e

	@yarn install

	@echo "Building the program..."
	@anchor build

test:
    @anchor build
    @cargo test -- --nocapture