(use-modules (gnu)
             (gnu bootloader grub)
             (gnu packages base)
             (gnu packages bash)
             (gnu packages ssh)
             (gnu services networking)
             (gnu services ssh)
             (gnu system nss))

(operating-system
  (host-name "guix-p2p-node-a")
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
  (packages
   (append
    (list bash hello openssh-sans-x)
    %base-packages))
  (services
   (append
    (list (service dhcpcd-service-type)
          (service openssh-service-type
                   (openssh-configuration
                    (openssh openssh-sans-x)
                    (port-number 22))))
    %base-services))
  (name-service-switch %mdns-host-lookup-nss))
