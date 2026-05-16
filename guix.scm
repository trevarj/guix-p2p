(use-modules (guix build-system gnu)
             (guix git-download)
             (guix gexp)
             ((guix licenses) #:prefix license:)
             (guix packages)
             (guix utils)
             (gnu packages compression)
             (gnu packages commencement)
             (gnu packages gnupg)
             (gnu packages linux)
             (gnu packages nss)
             (gnu packages perl)
             (gnu packages pkg-config)
             (gnu packages rust)
             (gnu packages tls))

(define-public guix-p2p
  (package
    (name "guix-p2p")
    (version "0.1.0")
    (source (local-file "." "guix-p2p-checkout"
                        #:recursive? #t
                        #:select? (git-predicate ".")))
    (build-system gnu-build-system)
    (arguments
     (list
      #:tests? #f
      #:modules '((guix build gnu-build-system)
                  (guix build utils))
      #:phases
      #~(modify-phases %standard-phases
          (delete 'configure)
          (replace 'build
            (lambda _
              (setenv "HOME" (getcwd))
              (invoke "cargo" "build" "--release" "--locked"
                      "--bin" "guix-p2p"
                      "--bin" "guix-p2p-wrapper")))
          (replace 'check
            (lambda* (#:key tests? #:allow-other-keys)
              (when tests?
                (setenv "HOME" (getcwd))
                (invoke "cargo" "test" "--locked"))))
          (replace 'install
            (lambda _
              (let ((bin (string-append #$output "/bin")))
                (mkdir-p bin)
                (install-file "target/release/guix-p2p" bin)
                (install-file "target/release/guix-p2p-wrapper" bin)))))))
    (native-inputs
     (list gcc-toolchain
           pkg-config
           perl
           rust
           (list rust "cargo")))
    (inputs
     (list libgcrypt
           nss-certs
           openssl
           zstd))
    (home-page "https://codeberg.org/trevarj/guix-p2p")
    (synopsis "P2P binary substitute distribution for GNU Guix")
    (description
     "guix-p2p is a libp2p daemon and Guix substitute relay.  It installs both
the @command{guix-p2p} daemon and @command{guix-p2p-wrapper} PATH wrapper.")
    (license license:gpl3+)))

guix-p2p
