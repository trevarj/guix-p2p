(use-modules (gnu)
             (gnu bootloader grub)
             (gnu packages base)
             (gnu packages bash)
             (gnu services)
             (gnu services networking)
             (gnu services shepherd)
             (gnu system nss)
             (guix gexp))

(define guix-p2p-e2e-runner
  (program-file
   "guix-p2p-e2e-runner"
   #~(begin
       (use-modules (ice-9 popen)
                    (ice-9 rdelim)
                    (ice-9 textual-ports))

       (define console (open-file "/dev/console" "a"))

       (define (emit text)
         (display text)
         (force-output)
         (display text console)
         (force-output console))

       (define (emit-line text)
         (emit text)
         (emit "\n"))

       (define (copy-output port)
         (let loop ((line (read-line port)))
           (unless (eof-object? line)
             (emit-line line)
             (loop (read-line port)))))

       (define (open-command args)
         ;; Route child stderr into stdout so failures are visible on the
         ;; serial console, not only in Shepherd's service log.
         (apply open-pipe* OPEN_READ "/run/current-system/profile/bin/sh" "-c"
                "exec \"$@\" 2>&1"
                "guix-p2p-e2e-runner"
                args))

       (define (run . args)
         (emit-line (format #f "running: ~s" args))
         (let* ((port (open-command args))
                (_ (copy-output port))
                (status (close-pipe port)))
           (unless (zero? status)
             (emit-line (format #f "command failed: ~s" args))
             (exit 1))))

       (define (try-run . args)
         (let* ((port (open-command args))
                (_ (copy-output port))
                (status (close-pipe port)))
           (zero? status)))

       (define target "/tmp/guix-p2p-target")
       (define base "/tmp/guix-p2p-e2e")
       (define payload "/mnt/guix-p2p-bin")
       (define payload-device "/dev/disk/by-label/guix-p2p-bin")
       (define profile "/run/current-system/profile/bin/")

       (run (string-append profile "mkdir") "-p" target payload)

       (let loop ((remaining 30))
         (cond
          ((try-run (string-append profile "test") "-e" payload-device) #t)
          ((zero? remaining)
           (emit-line "timed out waiting for guix-p2p payload disk")
           (try-run (string-append profile "ls") "-l" "/dev/disk/by-label")
           (exit 1))
          (else
           (try-run (string-append profile "sleep") "1")
           (loop (- remaining 1)))))

       (run (string-append profile "mount") "-o" "ro" payload-device payload)

       ;; This VM is a disposable test image. The raw test guix-daemon imports
       ;; substituted nars into /gnu/store, so the harness must be able to write.
       (try-run (string-append profile "mount") "-o" "remount,rw" "/gnu/store")
       (unless (try-run (string-append profile "test") "-w" "/gnu/store")
         (format (current-error-port)
                 "/gnu/store is not writable in the disposable E2E VM~%")
         (exit 1))

       (setenv "CARGO_TARGET_DIR" target)
       (setenv "GUIX_P2P_E2E_BASE" base)
       (setenv "LD_LIBRARY_PATH" (string-append payload "/lib"))
       (setenv "GUIX_P2P_E2E_NO_GUIX_SHELL" "1")
       (setenv "GUIX_P2P_E2E_REMOVE_SEED_AFTER_NODE_A" "1")
       (run (string-append payload "/bin/guix-p2p-e2e") "container-smoke"
            "--package" #$(raw-derivation-file hello)
            "--store-path" #$hello
            "--transport" "tcp"
            "--guix-p2p-bin" (string-append payload "/bin/guix-p2p")
            "--dashboard-bind" "0.0.0.0"
            "--hold"
            "--keep-temp"))))

(define guix-p2p-e2e-service
  (shepherd-service
   (provision '(guix-p2p-e2e))
   (requirement '(user-processes networking guix-daemon))
   (documentation "Run the guix-p2p disposable VM smoke proof.")
   (start #~(make-forkexec-constructor
             (list #$guix-p2p-e2e-runner)
             #:log-file "/var/log/guix-p2p-e2e.log"))
   (respawn? #f)
   (stop #~(make-kill-destructor))))

(operating-system
  (host-name "guix-p2p-e2e")
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
  (packages
   (append
    (list bash hello)
    %base-packages))
  (file-systems
   (cons (file-system
           (mount-point "/")
           (device (file-system-label "Guix_image"))
           (type "ext4"))
         %base-file-systems))
  (services
    (append
    (list (service dhcpcd-service-type)
          (simple-service 'guix-p2p-e2e-runner
                          shepherd-root-service-type
                          (list guix-p2p-e2e-service)))
    %base-services))
  (name-service-switch %mdns-host-lookup-nss))
