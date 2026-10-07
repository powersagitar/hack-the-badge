#!/usr/bin/env bash
# Vercel build step (see ../vercel.json). Vercel's build image has no Rust,
# so install a minimal toolchain plus a prebuilt wasm-pack, regenerate the
# gitignored src/cpu/wasm-pkg/, then run the normal Vite build.
set -euo pipefail

WASM_PACK_VERSION=0.15.0

if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --target wasm32-unknown-unknown
  . "$HOME/.cargo/env"
fi

if ! command -v wasm-pack >/dev/null; then
  mkdir -p "$HOME/.cargo/bin"
  export PATH="$HOME/.cargo/bin:$PATH"
  curl -sSfL "https://github.com/rustwasm/wasm-pack/releases/download/v${WASM_PACK_VERSION}/wasm-pack-v${WASM_PACK_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    | tar xz --strip-components=1 -C "$HOME/.cargo/bin" \
      "wasm-pack-v${WASM_PACK_VERSION}-x86_64-unknown-linux-musl/wasm-pack"
fi

if ! command -v bun >/dev/null; then
  curl -fsSL https://bun.sh/install | bash
  export PATH="$HOME/.bun/bin:$PATH"
fi

bun run build:wasm
bun run build
