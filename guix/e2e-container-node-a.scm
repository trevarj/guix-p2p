(use-modules (gnu)
             (gnu bootloader grub)
             (gnu packages bash)
             (gnu services)
             (gnu services shepherd)
             (guix gexp))

(define guix-p2p-node-a-runner
  (program-file
   "guix-p2p-node-a-runner"
   #~(begin
       (define project-dir "/src")
       (define binary (string-append project-dir "/target/release/guix-p2p"))
       (define cache-dir "/var/lib/guix-p2p-node-a")
       (define socket "/run/guix-p2p-node-a.sock")

       (unless (access? binary X_OK)
         (format (current-error-port)
                 "guix-p2p binary is missing or not executable: ~a~%"
                 binary)
         (exit 1))

       (execl binary binary
              "--daemon"
              "--listen-addr" "/ip4/127.0.0.1/tcp/6881"
              "--cache-dir" cache-dir
              "--socket" socket
              "--dashboard"
              "--dashboard-bind" "127.0.0.1"
              "--dashboard-port" "3031"
              "--policy" "p2p-only"))))

(define guix-p2p-node-a-service
  (shepherd-service
   (provision '(guix-p2p-node-a))
   (requirement '(user-processes))
   (documentation "Run guix-p2p Node A for container diagnosis.")
   (start #~(make-forkexec-constructor
             (list #$guix-p2p-node-a-runner)
             #:log-file "/var/log/guix-p2p-node-a.log"))
   (stop #~(make-kill-destructor))))

(operating-system
  (host-name "guix-p2p-node-a")
  (timezone "Etc/UTC")
  (locale "en_US.utf8")
  (bootloader
   (bootloader-configuration
    (bootloader grub-bootloader)
    (targets '("/dev/null"))))
  (file-systems %base-file-systems)
  (packages (cons bash %base-packages))
  (services
   (cons (simple-service 'guix-p2p-node-a-service
                         shepherd-root-service-type
                         (list guix-p2p-node-a-service))
         %base-services)))
