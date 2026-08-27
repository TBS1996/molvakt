{
  description = "WhatsApp language-learning bot";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
      in {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            # Rust
            cargo
            rustc
            rust-analyzer
            rustfmt
            clippy

            # Database
            sqlite
            sqlx-cli

            # Native dependencies commonly needed by Rust crates
            pkg-config
            openssl

            # Useful for testing the webhook
            curl
            jq

            # Expose localhost to Meta during development
            cloudflared
          ];

          env = {
            RUST_BACKTRACE = "1";

            # sqlx-cli will use this by default
            DATABASE_URL = "sqlite://languagebot.db";
          };

          shellHook = ''
            echo "ᚱ Language bot dev shell"
            echo "Rust: $(rustc --version)"
            echo "DB:   $DATABASE_URL"
          '';
        };
      }
    );
}
