(eval-when (expand load eval)
  (let ((root (dirname (current-filename))))
    (setenv "GUIX_P2P_CHECKOUT_ROOT" root)
    (add-to-load-path (string-append root "/channel"))))

(use-modules (guix-p2p packages)
             (guix-p2p services))

guix-p2p
