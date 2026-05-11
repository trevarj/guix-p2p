(use-modules (gnu)
             (guix build-system trivial)
             (guix gexp)
             ((guix licenses) #:prefix license:)
             (guix packages)
             (gnu bootloader grub)
             (gnu packages bash)
             (gnu packages commencement)
             (gnu packages curl)
             (gnu packages gnupg)
             (gnu packages ssh)
             (gnu packages tls)
             (gnu services networking)
             (gnu services ssh)
             (gnu system nss))

(define %guix-p2p-binary
  (local-file (or (getenv "GUIX_P2P_E2E_BINARY")
                  "target/release/guix-p2p")
              "guix-p2p-release"))

(define %ssh-host-key
  (local-file (or (getenv "GUIX_P2P_E2E_SSH_HOST_KEY")
                  "target/guix-p2p-e2e/ssh/ssh_host_ed25519_key")
              "ssh_host_ed25519_key"))

(define %ssh-host-key.pub
  (local-file (or (getenv "GUIX_P2P_E2E_SSH_HOST_KEY_PUB")
                  "target/guix-p2p-e2e/ssh/ssh_host_ed25519_key.pub")
              "ssh_host_ed25519_key.pub"))

(define %ssh-authorized-key
  (local-file (or (getenv "GUIX_P2P_E2E_SSH_AUTHORIZED_KEY")
                  "target/guix-p2p-e2e/ssh/e2e_ed25519.pub")
              "e2e_ed25519.pub"))

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
                 (wrapper (string-append bin "/guix-p2p"))
                 (node-a (string-append bin "/guix-p2p-e2e-node-a"))
                 (node-b (string-append bin "/guix-p2p-e2e-node-b")))
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
                  #$openssl "/lib:" #$libgcrypt "/lib:" #$gcc-toolchain "/lib"
                  "${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}\"\n"
                 "exec \"" real "\" \"$@\"\n")
                 port)))
            (chmod wrapper #o555)
            (call-with-output-file node-a
              (lambda (port)
                (display
                 (string-append
                  "#!" #$(file-append bash "/bin/sh") "\n"
                  "set -eu\n"
                  "export LD_LIBRARY_PATH=\""
                  #$openssl "/lib:" #$libgcrypt "/lib:" #$gcc-toolchain "/lib"
                  "${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}\"\n"
                  "PACKAGE=\"${1:-hello}\"\n"
                  "P2P=\"${GUIX_P2P_E2E_P2P_BIN:-guix-p2p}\"\n"
                  "CACHE_DIR=\"${GUIX_P2P_E2E_A_CACHE:-/tmp/guix-p2p-a}\"\n"
                  "LOG=\"${GUIX_P2P_E2E_A_LOG:-/tmp/guix-p2p-a.log}\"\n"
                  "SOCKET=\"${GUIX_P2P_E2E_A_SOCKET:-$CACHE_DIR/guix-p2p.sock}\"\n"
                  "SUBSTITUTE_URLS=\"${GUIX_P2P_E2E_SUBSTITUTE_URLS:-https://ci.guix.gnu.org https://bordeaux.guix.gnu.org}\"\n"
                  "LISTEN=\"${GUIX_P2P_E2E_A_LISTEN:-/ip4/0.0.0.0/tcp/6881}\"\n"
                  "DASHBOARD_BIND=\"${GUIX_P2P_E2E_A_DASHBOARD_BIND:-0.0.0.0}\"\n"
                  "DASHBOARD_PORT=\"${GUIX_P2P_E2E_A_DASHBOARD_PORT:-3031}\"\n"
                  "BOOTSTRAP=\"${GUIX_P2P_E2E_A_BOOTSTRAP:-}\"\n"
                  "mkdir -p \"$CACHE_DIR\" \"$HOME/.config/guix-p2p\"\n"
                  "printf 'min_providers = 1\\n' > \"$HOME/.config/guix-p2p/config.toml\"\n"
                  "STORE_PATH=\"$(guix build --no-grafts --substitute-urls=\"$SUBSTITUTE_URLS\" \"$PACKAGE\")\"\n"
                  "STORE_HASH=\"${STORE_PATH#/gnu/store/}\"\n"
                  "STORE_HASH=\"${STORE_HASH%%-*}\"\n"
                  "NARINFO_URL=''\n"
                  "for BASE_URL in $SUBSTITUTE_URLS; do\n"
                  "  URL=\"${BASE_URL%/}/$STORE_HASH.narinfo\"\n"
                  "  if curl -fsI \"$URL\" >/dev/null 2>&1; then\n"
                  "    NARINFO_URL=\"$URL\"\n"
                  "    break\n"
                  "  fi\n"
                  "done\n"
                  "if [ -z \"$NARINFO_URL\" ]; then\n"
                  "  echo \"no official narinfo found for $STORE_PATH\" >&2\n"
                  "  echo \"B needs signed narinfo from one of: $SUBSTITUTE_URLS\" >&2\n"
                  "  exit 1\n"
                  "fi\n"
                  "printf '%s\\n' \"$STORE_PATH\" > /tmp/guix-p2p-a-store-path\n"
                  "printf '%s\\n' \"$NARINFO_URL\" > /tmp/guix-p2p-a-narinfo-url\n"
                  "if [ -f /tmp/guix-p2p-a.pid ]; then\n"
                  "  OLD_PID=\"$(cat /tmp/guix-p2p-a.pid 2>/dev/null || true)\"\n"
                  "  if [ -n \"$OLD_PID\" ] && kill -0 \"$OLD_PID\" 2>/dev/null; then\n"
                  "    kill \"$OLD_PID\" 2>/dev/null || true\n"
                  "    sleep 1\n"
                  "  fi\n"
                  "fi\n"
                  "rm -f \"$SOCKET\"\n"
                  "BOOTSTRAP_ARGS=''\n"
                  "if [ -n \"$BOOTSTRAP\" ]; then\n"
                  "  BOOTSTRAP_ARGS=\"--bootstrap-peers $BOOTSTRAP\"\n"
                  "fi\n"
                  "RUST_LOG=\"${RUST_LOG:-info}\" \"$P2P\" --daemon \\\n"
                  "  --cache-dir \"$CACHE_DIR\" \\\n"
                  "  --listen-addr \"$LISTEN\" \\\n"
                  "  --socket \"$SOCKET\" \\\n"
                  "  --dashboard --dashboard-bind \"$DASHBOARD_BIND\" --dashboard-port \"$DASHBOARD_PORT\" \\\n"
                  "  $BOOTSTRAP_ARGS \\\n"
                  "  --seed \"$STORE_PATH\" \\\n"
                  "  > \"$LOG\" 2>&1 &\n"
                  "PID=\"$!\"\n"
                  "printf '%s\\n' \"$PID\" > /tmp/guix-p2p-a.pid\n"
                  "PEER_ID=''\n"
                  "i=0\n"
                  "while [ \"$i\" -lt 30 ]; do\n"
                  "  PEER_ID=\"$(sed -n 's/.*Peer ID: //p' \"$LOG\" 2>/dev/null | tail -n 1)\"\n"
                  "  [ -n \"$PEER_ID\" ] && break\n"
                  "  i=$((i + 1))\n"
                  "  sleep 1\n"
                  "done\n"
                  "[ -n \"$PEER_ID\" ] && printf '%s\\n' \"$PEER_ID\" > /tmp/guix-p2p-a-peer-id\n"
                  "printf 'store_path=%s\\n' \"$STORE_PATH\"\n"
                  "printf 'narinfo_url=%s\\n' \"$NARINFO_URL\"\n"
                  "printf 'peer_id=%s\\n' \"$PEER_ID\"\n"
                  "printf 'pid=%s\\nlog=%s\\nsocket=%s\\ndashboard=http://127.0.0.1:%s\\n' \"$PID\" \"$LOG\" \"$SOCKET\" \"$DASHBOARD_PORT\"\n")
                 port)))
            (chmod node-a #o555)
            (call-with-output-file node-b
              (lambda (port)
                (display
                 (string-append
                  "#!" #$(file-append bash "/bin/sh") "\n"
                  "set -eu\n"
                  "export LD_LIBRARY_PATH=\""
                  #$openssl "/lib:" #$libgcrypt "/lib:" #$gcc-toolchain "/lib"
                  "${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}\"\n"
                  "if [ \"$#\" -lt 2 ]; then\n"
                  "  echo 'usage: guix-p2p-e2e-node-b STORE_PATH NODE_A_PEER_ID' >&2\n"
                  "  exit 2\n"
                  "fi\n"
                  "STORE_PATH=\"$1\"\n"
                  "PEER_ID=\"$2\"\n"
                  "P2P=\"${GUIX_P2P_E2E_P2P_BIN:-guix-p2p}\"\n"
                  "CACHE_DIR=\"${GUIX_P2P_E2E_B_CACHE:-/tmp/guix-p2p-b}\"\n"
                  "LOG=\"${GUIX_P2P_E2E_B_LOG:-/tmp/guix-p2p-b.log}\"\n"
                  "SOCKET=\"${GUIX_P2P_E2E_B_SOCKET:-$CACHE_DIR/guix-p2p.sock}\"\n"
                  "LISTEN=\"${GUIX_P2P_E2E_B_LISTEN:-/ip4/0.0.0.0/tcp/6882}\"\n"
                  "DASHBOARD_BIND=\"${GUIX_P2P_E2E_B_DASHBOARD_BIND:-0.0.0.0}\"\n"
                  "DASHBOARD_PORT=\"${GUIX_P2P_E2E_B_DASHBOARD_PORT:-3032}\"\n"
                  "BOOTSTRAP=\"${GUIX_P2P_E2E_B_BOOTSTRAP:-/ip4/10.0.2.2/tcp/6881/p2p/$PEER_ID}\"\n"
                  "if [ -e \"$STORE_PATH\" ]; then\n"
                  "  echo \"fetch node already has $STORE_PATH; stop before mutating the proof\" >&2\n"
                  "  exit 1\n"
                  "fi\n"
                  "mkdir -p \"$CACHE_DIR\" \"$HOME/.config/guix-p2p\"\n"
                  "printf 'min_providers = 1\\n' > \"$HOME/.config/guix-p2p/config.toml\"\n"
                  "if [ -f /tmp/guix-p2p-b.pid ]; then\n"
                  "  OLD_PID=\"$(cat /tmp/guix-p2p-b.pid 2>/dev/null || true)\"\n"
                  "  if [ -n \"$OLD_PID\" ] && kill -0 \"$OLD_PID\" 2>/dev/null; then\n"
                  "    kill \"$OLD_PID\" 2>/dev/null || true\n"
                  "    sleep 1\n"
                  "  fi\n"
                  "fi\n"
                  "rm -f \"$SOCKET\"\n"
                  "RUST_LOG=\"${RUST_LOG:-info}\" \"$P2P\" --daemon \\\n"
                  "  --cache-dir \"$CACHE_DIR\" \\\n"
                  "  --listen-addr \"$LISTEN\" \\\n"
                  "  --socket \"$SOCKET\" \\\n"
                  "  --dashboard --dashboard-bind \"$DASHBOARD_BIND\" --dashboard-port \"$DASHBOARD_PORT\" \\\n"
                  "  --bootstrap-peers \"$BOOTSTRAP\" \\\n"
                  "  --policy p2p-only \\\n"
                  "  > \"$LOG\" 2>&1 &\n"
                  "PID=\"$!\"\n"
                  "printf '%s\\n' \"$PID\" > /tmp/guix-p2p-b.pid\n"
                  "i=0\n"
                  "while [ \"$i\" -lt 30 ]; do\n"
                  "  [ -S \"$SOCKET\" ] && break\n"
                  "  i=$((i + 1))\n"
                  "  sleep 1\n"
                  "done\n"
                  "printf 'store_path=%s\\n' \"$STORE_PATH\"\n"
                  "printf 'bootstrap=%s\\n' \"$BOOTSTRAP\"\n"
                  "printf 'pid=%s\\nlog=%s\\nsocket=%s\\ndashboard=http://127.0.0.1:%s\\n' \"$PID\" \"$LOG\" \"$SOCKET\" \"$DASHBOARD_PORT\"\n"
                  "if [ -S \"$SOCKET\" ]; then\n"
                  "  printf 'have_query=' && printf 'have %s\\n' \"$STORE_PATH\" | RUST_LOG=warn \"$P2P\" --query --socket \"$SOCKET\" 4>&1\n"
                  "else\n"
                  "  echo \"socket did not appear yet; inspect $LOG\" >&2\n"
                  "fi\n")
                 port)))
            (chmod node-b #o555)))))
    (home-page "https://example.invalid/guix-p2p-e2e")
    (synopsis "Locally built guix-p2p binary for E2E images")
    (description "This package wraps the locally built guix-p2p binary for the
two-node E2E VM image.")
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
  ;; Keep the base image neutral. Scenario commands decide which node seeds
  ;; and which node fetches the package under test.
  (packages
   (append
    (list bash curl gcc-toolchain %guix-p2p-e2e-package openssh-sans-x openssl)
    %base-packages))
  (services
   (append
    (list (simple-service 'guix-p2p-e2e-ssh-host-key
                          activation-service-type
                          #~(begin
                              (use-modules (guix build utils))
                              (mkdir-p "/etc/ssh")
                              (copy-file #$%ssh-host-key
                                         "/etc/ssh/ssh_host_ed25519_key")
                              (copy-file #$%ssh-host-key.pub
                                         "/etc/ssh/ssh_host_ed25519_key.pub")
                              (chmod "/etc/ssh/ssh_host_ed25519_key" #o600)
                              (chmod "/etc/ssh/ssh_host_ed25519_key.pub" #o644)))
          (service dhcpcd-service-type)
          (service openssh-service-type
                   (openssh-configuration
                    (openssh openssh-sans-x)
                    (authorized-keys
                     `(("e2e" ,%ssh-authorized-key)))
                    (generate-host-keys? #f)
                    (password-authentication? #t)
                    (port-number 22))))
    %base-services))
  (name-service-switch %mdns-host-lookup-nss))
