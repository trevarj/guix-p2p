(use-modules (gnu)
             (gnu bootloader grub))

(operating-system
  (host-name "guix-p2p-minimal")
  (timezone "Etc/UTC")
  (locale "en_US.utf8")
  (bootloader
   (bootloader-configuration
    (bootloader grub-bootloader)
    (targets '("/dev/null"))))
  (file-systems %base-file-systems)
  (packages %base-packages)
  (services %base-services))
