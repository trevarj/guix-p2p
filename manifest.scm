(use-modules (guix packages)
             (gnu)
             (gnu packages base)
             (gnu packages commencement)
             (gnu packages crypto)
             (gnu packages linux)
             (gnu packages llvm)
             (gnu packages node)
             (gnu packages rust)
             (gnu packages tls)
             (gnu packages perl)
             (gnu packages pkg-config)
             (gnu packages python)
             (gnu packages python-build)
             (gnu packages python-xyz)
             (gnu packages sqlite)
             (gnu packages xorg)
             (rustup build toolchain))

(packages->manifest
 (list
  gcc-toolchain
  gnu-make
  clang-toolchain-21
  binutils
  pkg-config

  ;; Rustup
  (rustup "nightly-2026-03-14"
          #:components
          '("rust-analyzer" "rustfmt" "rust-src" "clippy"))

  ;; Libraries for certain sys crates
  openssl
  eudev                                 ; libudev replacement
  libsecp256k1

  ;; WASM stuff
  node

  ;; Extras
  perl
  python
  python-pip
  python-virtualenv
  sqlite
  ))
