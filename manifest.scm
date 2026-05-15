(use-modules (guix packages)
             (gnu)
             (gnu packages base)
             (gnu packages commencement)
             (gnu packages crypto)
             (gnu packages gnupg)
             (gnu packages linux)
             (gnu packages llvm)
             (gnu packages libunwind)
             (gnu packages musl)
             (gnu packages node)
             (gnu packages nss)
             (gnu packages rust)
             (gnu packages tls)
             (gnu packages perl)
             (gnu packages pkg-config)
             (gnu packages python)
             (gnu packages python-build)
             (gnu packages python-xyz)
             (gnu packages sqlite)
             (gnu packages xorg))

(packages->manifest
 (list
  gcc-toolchain
  gnu-make
  clang-toolchain-21
  binutils
  libunwind
  musl
  pkg-config
  nss-certs

  ;; Rustup
  rust
  (list rust "cargo")
  (list rust "tools")
  (list rust "rust-src")

  ;; Libraries for certain sys crates
  openssl
  libgcrypt
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
