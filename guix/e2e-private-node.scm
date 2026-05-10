(use-modules (gnu)
             (guix build-system trivial)
             (guix gexp)
             ((guix licenses) #:prefix license:)
             (guix packages)
             (gnu bootloader grub)
             (gnu packages bash)
             (gnu packages commencement)
             (gnu packages ssh)
             (gnu packages tls)
             (gnu services networking)
             (gnu services ssh)
             (gnu system nss))

(define %guix-p2p-binary
  (local-file (or (getenv "GUIX_P2P_E2E_BINARY")
                  "target/release/guix-p2p")
              "guix-p2p-release"))

(define %guix-p2p-e2e-package
  (package
    (name "guix-p2p-e2e")
    (version "0")
    (source %guix-p2p-binary)
    (build-system trivial-build-system)
    (arguments
     (list
      #:modules '((guix build utils))
      #:builder
      #~(begin
          (use-modules (guix build utils))
          (let* ((bin (string-append #$output "/bin"))
                 (real (string-append bin "/.guix-p2p-real"))
                 (wrapper (string-append bin "/guix-p2p")))
            (mkdir-p bin)
            (copy-file #$%guix-p2p-binary real)
            (chmod real #o555)
            ;; The e2e image embeds the locally built Rust binary. Keep the
            ;; runtime libraries visible without a full Rust package yet.
            (call-with-output-file wrapper
              (lambda (port)
                (display
                 (string-append
                  "#!" #$(file-append bash "/bin/sh") "\n"
                  "export LD_LIBRARY_PATH=\""
                  #$openssl "/lib:" #$gcc-toolchain "/lib"
                  "${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}\"\n"
                  "exec \"" real "\" \"$@\"\n")
                 port)))
            (chmod wrapper #o555)))))
    (home-page "https://example.invalid/guix-p2p-e2e")
    (synopsis "Locally built guix-p2p binary for private-store E2E images")
    (description "This package wraps the locally built guix-p2p binary for the
private-store E2E VM image.")
    (license license:expat)))

(operating-system
  (host-name "guix-p2p-node")
  (timezone "Etc/UTC")
  (locale "en_US.utf8")
  (bootloader
   (bootloader-configuration
    (bootloader grub-bootloader)
    (targets '("/dev/vda"))
    (timeout 1)
    (terminal-outputs '(serial))
    (terminal-inputs '(serial))
    (serial-unit 0)
    (serial-speed 115200)))
  (kernel-arguments '("console=ttyS0,115200n8"))
  (file-systems
   (cons (file-system
           (mount-point "/")
           (device (file-system-label "Guix_image"))
           (type "ext4"))
         %base-file-systems))
  (users (cons (user-account
                (name "e2e")
                (comment "E2E test user")
                (password (crypt "e2e" "$6$e2e"))
                (group "users")
                (supplementary-groups '("wheel" "netdev")))
               %base-user-accounts))
  ;; Keep the base image neutral. Node A will realize the package under test
  ;; after boot so Node B starts from an identical store without that package.
  (packages
   (append
    (list bash gcc-toolchain %guix-p2p-e2e-package openssh-sans-x openssl)
    %base-packages))
  (services
   (append
    (list (service dhcpcd-service-type)
          (service openssh-service-type
                   (openssh-configuration
                   (openssh openssh-sans-x)
                    (password-authentication? #t)
                    (port-number 22))))
    %base-services))
  (name-service-switch %mdns-host-lookup-nss))
