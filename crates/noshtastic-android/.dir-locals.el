((rustic-mode
  .
  ((eval . (make-local-variable 'process-environment))
   ;; Buffers outside this directory won't see this change because their
   ;; process-environment remains the global one, unaffected by your
   ;; directory-local settings.
   (eval . (setenv "CARGO_BUILD_TARGET" "aarch64-linux-android"))
   (eval . (setq rustic-cargo-build-arguments '("--target" "aarch64-linux-android"))))))
